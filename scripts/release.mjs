// Prepares a release: signs the installer, writes the update manifest and checks the result the way
// the island will. Nothing is uploaded: the files are left in `release/` for `gh release create`.
//
//   npx tauri build
//   node scripts/release.mjs
//
// The private key is read from %USERPROFILE%\.great-island\update.key (or `--key`); its password is
// asked for, or taken from TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
//
// Options (for tests): --installer <file>  --out <dir>  --key <file>  --public-key <base64>
import { execSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = 'NureShumeikoAnton/great-island';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const options = {};
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i += 2) {
  if (!argv[i].startsWith('--') || argv[i + 1] === undefined) fail(`unknown argument: ${argv[i]}`);
  options[argv[i].slice(2)] = argv[i + 1];
}

function fail(message) {
  console.error(`release: ${message}`);
  process.exit(1);
}

const read = (...parts) => fs.readFileSync(path.join(root, ...parts), 'utf8');

// One version in all the places it lives.
const version = JSON.parse(read('package.json')).version;
const tauriVersion = JSON.parse(read('src-tauri', 'tauri.conf.json')).version;
const cargoVersion = /^version = "([^"]+)"/m.exec(read('src-tauri', 'Cargo.toml'))?.[1];
if (tauriVersion !== version || cargoVersion !== version) {
  fail(`versions differ: package.json ${version}, tauri.conf.json ${tauriVersion}, Cargo.toml ${cargoVersion}`);
}

// The text under "## <version> — date" of a changelog, without the heading.
function notesOf(file) {
  const lines = read(file).split(/\r?\n/);
  const start = lines.findIndex((line) => line.startsWith(`## ${version} `) || line === `## ${version}`);
  if (start < 0) fail(`${file} has no section for ${version}: move "Не выпущено"/"Unreleased" under the version first`);
  let end = lines.findIndex((line, i) => i > start && line.startsWith('## '));
  if (end < 0) end = lines.length;
  const text = lines.slice(start + 1, end).join('\n').trim();
  if (!text) fail(`the ${version} section of ${file} is empty`);
  return text;
}
const notes = { en: notesOf('CHANGELOG.en.md'), ru: notesOf('CHANGELOG.md') };

const installer =
  options.installer ?? path.join(root, 'src-tauri', 'target', 'release', 'bundle', 'nsis', `Great Island_${version}_x64-setup.exe`);
if (!fs.existsSync(installer)) fail(`no installer: ${installer} (run "npx tauri build")`);

const keyFile = options.key ?? path.join(os.homedir(), '.great-island', 'update.key');
if (!fs.existsSync(keyFile)) fail(`no private key: ${keyFile}`);

const out = path.resolve(options.out ?? path.join(root, 'release'));
fs.mkdirSync(out, { recursive: true });

// GitHub turns the spaces of an uploaded file name into dots: the name is made safe here so that the
// address in the manifest is the address of the asset.
const name = `Great-Island_${version}_x64-setup.exe`;
const file = path.join(out, name);
fs.copyFileSync(installer, file);
fs.rmSync(`${file}.sig`, { force: true });

console.log(`Signing ${name} with ${keyFile}`);
execSync(`npx tauri signer sign -f "${keyFile}" "${file}"`, { cwd: root, stdio: 'inherit' });
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
fs.writeFileSync(path.join(out, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
fs.writeFileSync(path.join(out, 'notes.md'), `## English\n\n${notes.en}\n\n## Русский\n\n${notes.ru}\n`);

// The island's own check: a wrong key or a damaged file stops the release here, not on the users' machines.
console.log('Checking the signature the way the island does');
const check = [`"${file}"`, `"${file}.sig"`, ...(options['public-key'] ? [options['public-key']] : [])].join(' ');
execSync(`cargo run --quiet --manifest-path src-tauri/Cargo.toml --example verify_release -- ${check}`, {
  cwd: root,
  stdio: 'inherit',
});

console.log(`\nReady in ${out}:`);
console.log(`  ${name}\n  latest.json\n  notes.md\n`);
console.log('Upload (after the commit is pushed and tagged):');
console.log(
  `  gh release create v${version} "${file}" "${path.join(out, 'latest.json')}" --repo ${REPO} --title "Great Island ${version}" --notes-file "${path.join(out, 'notes.md')}"`,
);
