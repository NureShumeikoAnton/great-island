// A whole release in one go, from the repository root:
//
//   node scripts/release.mjs "Great Island 0.7.2"
//
// The argument is the commit message. The script asks for the version and the key password, then bumps
// the version, runs the checks, builds and signs the installer, commits, pushes, tags and publishes the
// release on GitHub. How it works and what each step means: RELEASE.md (local, not in the repository).
//
// The private key is %USERPROFILE%\.great-island\update.key. Its password is asked for (hidden) or taken from
// TAURI_SIGNING_PRIVATE_KEY_PASSWORD. It is only passed to the signer through the environment.
//
// Options (for tests): --dry-run  --installer <file>  --out <dir>  --key <file>  --public-key <base64>
//   --dry-run: asks, then only signs and checks into --out; no file of the project changes, nothing is
//   built, committed or uploaded.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';

const REPO = 'NureShumeikoAnton/great-island';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const IS_WINDOWS = process.platform === 'win32';

// ---- arguments
const options = {};
const positional = [];
{
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith('--')) positional.push(argv[i]);
    else if (argv[i] === '--dry-run') options.dryRun = true;
    else if (argv[i + 1] === undefined) fail(`у параметра ${argv[i]} нет значения`);
    else options[argv[i].slice(2)] = argv[++i];
  }
}
const message = positional.join(' ').trim();
if (!message) {
  fail('нужно сообщение коммита: node scripts/release.mjs "Great Island 0.7.2"');
}

function fail(text) {
  console.error(`\nrelease: ${text}`);
  process.exit(1);
}

const step = (text) => console.log(`\n== ${text}`);
const read = (...parts) => fs.readFileSync(path.join(root, ...parts), 'utf8');
const write = (text, ...parts) => fs.writeFileSync(path.join(root, ...parts), text);

/** Runs a program; stops the release when it fails. `shell` is for npx and other .cmd files on Windows. */
function run(command, args, { env, capture = false, shell = false, allowFail = false } = {}) {
  const spawnOptions = {
    cwd: root,
    env: { ...process.env, ...env },
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    encoding: 'utf8',
  };
  // With a shell the call is one line (Node warns about arguments passed beside a shell): a path with a
  // space must be quoted by hand.
  const result = shell
    ? spawnSync([command, ...args].map((a) => (/\s/.test(a) ? `"${a}"` : a)).join(' '), { ...spawnOptions, shell: true })
    : spawnSync(command, args, spawnOptions);
  if (result.status !== 0 && !allowFail) {
    fail(`не получилось: ${command} ${args.join(' ')}`);
  }
  return result;
}
const git = (...args) => run('git', args, { capture: true, allowFail: true });
const gitOut = (...args) => (git(...args).stdout ?? '').trim();

// ---- questions
// On a terminal each question is a readline; piped input (tests) is read whole, line by line.
let pipedLines = null;
async function line(prompt, echo = true) {
  if (!process.stdin.isTTY) {
    pipedLines ??= fs.readFileSync(0, 'utf8').split(/\r?\n/);
    process.stdout.write(prompt);
    const answer = pipedLines.shift() ?? '';
    console.log(echo ? answer : '');
    return answer;
  }
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  return new Promise((resolve) => rl.question(prompt, (answer) => (rl.close(), resolve(answer))));
}

async function secret(prompt) {
  if (!process.stdin.isTTY) return line(prompt, false);
  process.stdout.write(prompt);
  return new Promise((resolve) => {
    let typed = '';
    process.stdin.setRawMode(true);
    process.stdin.resume();
    const onData = (chunk) => {
      for (const ch of chunk.toString('utf8')) {
        if (ch === '\r' || ch === '\n') {
          process.stdin.setRawMode(false);
          process.stdin.pause();
          process.stdin.off('data', onData);
          process.stdout.write('\n');
          return resolve(typed);
        }
        if (ch === '\u0003') process.exit(130);
        typed = ch === '\u007f' || ch === '\b' ? typed.slice(0, -1) : typed + ch;
      }
    };
    process.stdin.on('data', onData);
  });
}

// ---- versions
const SEMVER = /^\d+\.\d+\.\d+$/;
const parts = (v) => v.split('.').map(Number);
const newer = (a, b) => {
  const [x, y] = [parts(a), parts(b)];
  return x[0] - y[0] || x[1] - y[1] || x[2] - y[2];
};

function versionsInFiles() {
  return {
    'package.json': /"version":\s*"([^"]+)"/.exec(read('package.json'))?.[1],
    'src-tauri/tauri.conf.json': /"version":\s*"([^"]+)"/.exec(read('src-tauri', 'tauri.conf.json'))?.[1],
    'src-tauri/Cargo.toml': /^version = "([^"]+)"/m.exec(read('src-tauri', 'Cargo.toml'))?.[1],
  };
}

function lastReleased() {
  const tags = gitOut('tag', '--list', 'v*.*.*')
    .split(/\r?\n/)
    .map((t) => t.slice(1))
    .filter((v) => SEMVER.test(v))
    .sort(newer);
  return tags.at(-1) ?? null;
}

/** Puts `to` where `from` stands in the project's version fields. */
function bump(from, to) {
  const edit = (text, pattern, replacement, what) => {
    if (!pattern.test(text)) fail(`не нашёл версию ${from} в ${what}`);
    return text.replace(pattern, replacement);
  };
  const files = [
    [['package.json'], /("version":\s*")[^"]+(")/, 'package.json'],
    [['src-tauri', 'tauri.conf.json'], /("version":\s*")[^"]+(")/, 'tauri.conf.json'],
    [['src-tauri', 'Cargo.toml'], /^(version = ")[^"]+(")/m, 'Cargo.toml'],
    [['src-tauri', 'Cargo.lock'], /(name = "great-island"\r?\nversion = ")[^"]+(")/, 'Cargo.lock'],
  ];
  for (const [file, pattern, what] of files) {
    write(edit(read(...file), pattern, `$1${to}$2`, what), ...file);
  }
  // package-lock.json keeps the version twice: the package and its root entry.
  let lock = read('package-lock.json');
  const twice = new RegExp(`("name": "great-island",\\s*"version": ")${from}(")`, 'g');
  if ((lock.match(twice) ?? []).length !== 2) fail('не нашёл версию в package-lock.json');
  write(lock.replace(twice, `$1${to}$2`), 'package-lock.json');
}

// ---- changelogs
function sectionOf(file, version) {
  const lines = read(file).split(/\r?\n/);
  const start = lines.findIndex((l) => l.startsWith(`## ${version} `) || l === `## ${version}`);
  if (start < 0) return null;
  let end = lines.findIndex((l, i) => i > start && l.startsWith('## '));
  if (end < 0) end = lines.length;
  return lines.slice(start + 1, end).join('\n').trim();
}

/** Moves what is under "Unreleased" under the new version. Nothing there is a mistake. */
function releaseChangelog(file, heading, version, date) {
  if (sectionOf(file, version) !== null) return;
  const text = read(file);
  const eol = text.includes('\r\n') ? '\r\n' : '\n';
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((l) => l === `## ${heading}`);
  if (start < 0) fail(`в ${file} нет раздела «${heading}»`);
  let end = lines.findIndex((l, i) => i > start && l.startsWith('## '));
  if (end < 0) end = lines.length;
  if (!lines.slice(start + 1, end).join('').trim()) {
    fail(`в ${file} под «${heading}» пусто: нечего выпускать. Сначала запишите, что изменилось`);
  }
  lines.splice(start + 1, 0, '', `## ${version} — ${date}`);
  write(lines.join(eol), file);
}

// ---- the questions and the checks that need no changes yet
console.log(`Релиз Great Island. Сообщение коммита: «${message}»`);
step('Проверяю, что всё на месте');

if (options.dryRun) console.log('(пробный прогон: файлы проекта не меняются, ничего не собирается и не загружается)');

const keyFile = options.key ?? path.join(os.homedir(), '.great-island', 'update.key');
if (!fs.existsSync(keyFile)) fail(`нет приватного ключа: ${keyFile}`);

if (!options.dryRun) {
  if (gitOut('rev-parse', '--abbrev-ref', 'HEAD') !== 'main') fail('релиз делается из ветки main');
  if (!gitOut('remote', 'get-url', 'origin').includes(REPO)) fail(`origin не ${REPO}`);
  if (run('gh', ['auth', 'status'], { capture: true, allowFail: true }).status !== 0) {
    fail('gh не авторизован: выполните «gh auth login»');
  }
  git('fetch', 'origin', '--tags');
  if (Number(gitOut('rev-list', '--count', 'HEAD..origin/main')) > 0) {
    fail('на GitHub есть коммиты, которых у вас нет: сначала git pull');
  }
}

const found = versionsInFiles();
const inFiles = found['package.json'];
if (new Set(Object.values(found)).size !== 1) fail(`версии в файлах разные: ${JSON.stringify(found)}`);
const released = lastReleased();
const suggested =
  released === null || newer(inFiles, released) > 0
    ? inFiles
    : `${parts(released)[0]}.${parts(released)[1]}.${parts(released)[2] + 1}`;

console.log(`\nВерсия в файлах: ${inFiles}. Последний релиз: ${released ? `v${released}` : 'нет'}.`);
console.log('Версия вида x.y.z: исправление — последняя цифра, новая возможность — средняя.');
let version;
if (options.dryRun) {
  version = inFiles;
  console.log(`Пробный прогон: версия ${version}`);
} else {
  for (;;) {
    const answer = (await line(`Какую версию выпускаем? [Enter = ${suggested}] `)).trim() || suggested;
    if (!SEMVER.test(answer)) console.log('  Нужно три числа через точки, например 0.7.2.');
    else if (released && newer(answer, released) <= 0) console.log(`  Версия должна быть новее v${released}.`);
    else {
      version = answer;
      break;
    }
  }
}

console.log('\nПароль от ключа подписи нужен, чтобы подписать установщик: без подписи приложение у пользователей');
console.log('его не примет. Пароль нигде не сохраняется и на экране не показывается.');
const password =
  process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? (await secret('Пароль ключа (Enter, если пароля нет): '));
const signEnv = { TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password };

// A wrong password is found now, not after a three-minute build.
const probeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'great-island-sign-'));
const probe = path.join(probeDir, 'probe.bin');
fs.writeFileSync(probe, 'probe');
const probed = run('npx', ['tauri', 'signer', 'sign', '-f', keyFile, probe], {
  env: signEnv,
  capture: true,
  shell: IS_WINDOWS,
  allowFail: true,
});
fs.rmSync(probeDir, { recursive: true, force: true });
if (probed.status !== 0) fail('ключ не открылся: неверный пароль или повреждён файл ключа');
console.log('Ключ открылся.');

const date = new Date().toISOString().slice(0, 10);
if (!options.dryRun) {
  console.log('\nИтог перед стартом:');
  console.log(`  версия:    ${version}  (в файлах сейчас ${inFiles})`);
  console.log(`  коммит:    «${message}»`);
  console.log(`  тег:       v${version}`);
  console.log('  что попадёт в коммит (git add -A):');
  const status = gitOut('status', '--short');
  console.log(status ? status.replace(/^/gm, '    ') : '    (ничего нового)');
  const answer = (await line('\nНачинаем? Дальше коммит, push и публикация релиза. [y/N] ')).trim().toLowerCase();
  if (answer !== 'y' && answer !== 'д') fail('остановлено: ничего не менялось и не публиковалось');
}

// ---- the work
const installerName = `Great Island_${version}_x64-setup.exe`;
const installer =
  options.installer ?? path.join(root, 'src-tauri', 'target', 'release', 'bundle', 'nsis', installerName);

if (!options.dryRun) {
  step(`Версия ${version} в файлах и в списках изменений`);
  if (version !== inFiles) bump(inFiles, version);
  releaseChangelog('CHANGELOG.md', 'Не выпущено', version, date);
  releaseChangelog('CHANGELOG.en.md', 'Unreleased', version, date);

  step('Проверки: cargo test, clippy, tsc');
  run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--quiet']);
  run('cargo', ['clippy', '--manifest-path', 'src-tauri/Cargo.toml', '--all-targets', '--quiet', '--', '-D', 'warnings']);
  run('npx', ['tsc', '--noEmit'], { shell: IS_WINDOWS });

  step('Сборка установщика (несколько минут)');
  const built = run('npx', ['tauri', 'build'], { shell: IS_WINDOWS, allowFail: true });
  if (built.status !== 0) {
    fail(
      'сборка не удалась. Если «Отказано в доступе»: остров запущен из src-tauri\\target\\release. ' +
        'Закройте его через трей или переименуйте exe и запустите скрипт снова',
    );
  }
}
if (!fs.existsSync(installer)) fail(`нет установщика: ${installer}`);

const notes = {
  en: sectionOf('CHANGELOG.en.md', version),
  ru: sectionOf('CHANGELOG.md', version),
};
if (!notes.en || !notes.ru) fail(`в списках изменений нет текста для ${version}`);

step('Подпись, манифест и проверка подписи');
const out = path.resolve(options.out ?? path.join(root, 'release'));
fs.mkdirSync(out, { recursive: true });
// GitHub turns the spaces of an uploaded file name into dots: the name is made safe here so that the
// address in the manifest is the address of the asset.
const name = `Great-Island_${version}_x64-setup.exe`;
const file = path.join(out, name);
fs.copyFileSync(installer, file);
fs.rmSync(`${file}.sig`, { force: true });
run('npx', ['tauri', 'signer', 'sign', '-f', keyFile, file], { env: signEnv, shell: IS_WINDOWS });
const signature = fs.readFileSync(`${file}.sig`, 'utf8').trim();

const manifest = {
  version,
  pub_date: new Date().toISOString(),
  notes,
  installer: {
    url: `https://github.com/${REPO}/releases/download/v${version}/${name}`,
    signature,
    size: fs.statSync(file).size,
  },
};
const manifestFile = path.join(out, 'latest.json');
const notesFile = path.join(out, 'notes.md');
fs.writeFileSync(manifestFile, `${JSON.stringify(manifest, null, 2)}\n`);
fs.writeFileSync(notesFile, `## English\n\n${notes.en}\n\n## Русский\n\n${notes.ru}\n`);

// The island's own check: a wrong key or a damaged file stops the release here, not on the users' machines.
run('cargo', [
  'run', '--quiet', '--manifest-path', 'src-tauri/Cargo.toml', '--example', 'verify_release', '--',
  file, `${file}.sig`, ...(options['public-key'] ? [options['public-key']] : []),
]);

if (options.dryRun) {
  console.log(`\nПробный прогон закончен. Файлы: ${out}`);
  process.exit(0);
}

step('Коммит, push и тег');
run('git', ['add', '-A']);
// Nothing to commit is fine: the changes may have been committed by hand before the release.
if (git('diff', '--cached', '--quiet').status !== 0) run('git', ['commit', '-m', message]);
run('git', ['push', 'origin', 'main']);
run('git', ['tag', `v${version}`]);
run('git', ['push', 'origin', `v${version}`]);

step('Релиз на GitHub');
const created = run(
  'gh',
  ['release', 'create', `v${version}`, file, manifestFile, '--repo', REPO, '--title', `Great Island ${version}`, '--notes-file', notesFile],
  { allowFail: true },
);
if (created.status !== 0) {
  fail(
    `коммит и тег уже на GitHub, а релиз не создался. Повторите только его:\n` +
      `  gh release create v${version} "${file}" "${manifestFile}" --repo ${REPO} --title "Great Island ${version}" --notes-file "${notesFile}"`,
  );
}

// GitHub serves "latest" a little late: the island that checks in these minutes still sees the old release.
step('Жду, пока GitHub начнёт отдавать новый манифест');
const url = `https://github.com/${REPO}/releases/latest/download/latest.json`;
let live = false;
for (let i = 0; i < 24 && !live; i++) {
  try {
    const served = await (await fetch(url, { cache: 'no-store' })).json();
    live = served.version === version;
  } catch {
    // Not served yet: wait and ask again.
  }
  if (!live) await new Promise((resolve) => setTimeout(resolve, 10_000));
}
console.log(
  live
    ? `\nГотово: релиз v${version} опубликован, манифест отдаётся.`
    : `\nРелиз v${version} опубликован, но манифест пока старый: GitHub догонит за несколько минут. Это не ошибка.`,
);
