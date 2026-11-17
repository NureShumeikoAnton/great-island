//! The sound inputs of a recording on Windows (WASAPI): what the computer plays, every
//! program but the island, and the chosen microphone (or the Windows default one).
//!
//! One thread reads both and hands packets over as they come, each stamped with the
//! moment its first frame was heard: the performance counter, the clock the recording
//! uses too. Windows converts every input to 48 kHz stereo 16 bit for us.
//!
//! Opening a device and reading its packets is shared with `microphones.rs`.

use crate::{
    fault::Failed,
    record::sound::{CHANNELS, Input, RATE, Sound},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
            AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
            ActivateAudioInterfaceAsync, DEVICE_STATE_ACTIVE, EDataFlow,
            IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
            IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
            PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVE_FORMAT_PCM, WAVEFORMATEX, eCapture,
            eConsole, eRender,
        },
        System::{
            Com::{
                BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, StructuredStorage::PROPVARIANT,
            },
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            Threading::{CreateEventW, GetCurrentProcessId, WaitForMultipleObjects},
            Variant::VT_BLOB,
        },
    },
    core::{Error, HRESULT, HSTRING, IUnknown, Interface, Ref, Result},
};
use windows_core::implement;

/// Room for 200 ms in Windows' buffer: the thread reads at least every `WAIT`.
const BUFFER: i64 = 2_000_000;
pub(super) const WAIT_MS: u32 = 100;
const RETRY: Duration = Duration::from_secs(1);
const ACTIVATION: Duration = Duration::from_secs(2);

pub enum Heard {
    /// `at` — when the first frame was heard, in 100 ns ticks of [`ticks`].
    Packet {
        input: Input,
        at: u64,
        samples: Vec<i16>,
    },
    Note(Note),
}

pub enum Note {
    /// Said once until the input is back.
    Lost(Input, Failed),
    Back(Input),
    /// This Windows can't leave the island's own sounds out: they are recorded too.
    WithIsland(Failed),
}

/// The performance counter in 100 ns ticks: the unit WASAPI stamps its packets in.
pub fn ticks() -> u64 {
    let (mut count, mut frequency) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut count);
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    (count as u128 * 10_000_000 / frequency.max(1) as u128) as u64
}

/// `microphone` — the chosen one; when it isn't plugged in, the Windows default is checked.
pub fn microphone_ready(microphone: Option<&str>) -> std::result::Result<(), Failed> {
    let (device, _) = microphone_device(microphone)?;
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    initialize(
        &client,
        AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    )?;
    Ok(())
}

/// The running inputs. Dropping it stops them.
pub struct Listening {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listening {
    pub fn start(
        sound: Sound,
        microphone: Option<String>,
        deliver: impl Fn(Heard) + Send + 'static,
    ) -> std::result::Result<Self, Failed> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("record-sound".into())
            .spawn(move || listen(sound, microphone, &flag, &deliver))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn listen(sound: Sound, microphone: Option<String>, stop: &AtomicBool, deliver: &dyn Fn(Heard)) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut streams: Vec<Stream> = [
        sound.system().then(|| Stream::new(Input::System, None)),
        sound
            .microphone()
            .then(|| Stream::new(Input::Microphone, microphone)),
    ]
    .into_iter()
    .flatten()
    .collect();
    while !stop.load(Ordering::Acquire) {
        for stream in &mut streams {
            stream.keep(deliver);
        }
        let events: Vec<HANDLE> = streams
            .iter()
            .filter_map(|s| s.open.as_ref().map(|o| o.event))
            .collect();
        if events.is_empty() {
            std::thread::sleep(Duration::from_millis(u64::from(WAIT_MS)));
            continue;
        }
        // Loopback may never signal its event on older Windows: the timeout reads anyway.
        unsafe {
            WaitForMultipleObjects(&events, false, WAIT_MS);
        }
        for stream in &mut streams {
            stream.read(deliver);
        }
    }
    drop(streams);
    unsafe { CoUninitialize() };
}

pub(super) struct Opened {
    client: IAudioClient,
    pub(super) capture: IAudioCaptureClient,
    pub(super) event: HANDLE,
    /// The endpoint it reads; `None` — every program's sound, not tied to a device.
    device: Option<String>,
}

impl Drop for Opened {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

struct Stream {
    input: Input,
    /// The chosen microphone; while it is unplugged the Windows default stands in.
    wanted: Option<String>,
    open: Option<Opened>,
    next_try: Instant,
    checked: Instant,
    lost: bool,
    /// Leaving the island out was refused once: this Windows can't, don't ask again.
    whole_system: bool,
}

impl Stream {
    fn new(input: Input, wanted: Option<String>) -> Self {
        Self {
            input,
            wanted,
            open: None,
            next_try: Instant::now(),
            checked: Instant::now(),
            lost: false,
            whole_system: false,
        }
    }

    /// Opens a closed input when it is time to try, and follows the device it should read:
    /// the default one, or the chosen microphone once it is back.
    fn keep(&mut self, deliver: &dyn Fn(Heard)) {
        let now = Instant::now();
        if let Some(open) = &self.open {
            if now.duration_since(self.checked) < RETRY {
                return;
            }
            self.checked = now;
            let Some(device) = &open.device else {
                return;
            };
            let current = self.device().ok().map(|(_, id)| id);
            if current.as_ref() == Some(device) {
                return;
            }
            self.open = None;
        }
        if now < self.next_try {
            return;
        }
        match self.connect(deliver) {
            Ok(opened) => {
                self.open = Some(opened);
                self.checked = now;
                if std::mem::take(&mut self.lost) {
                    deliver(Heard::Note(Note::Back(self.input)));
                }
            }
            Err(failed) => {
                self.next_try = now + RETRY;
                self.lose(failed, deliver);
            }
        }
    }

    fn lose(&mut self, failed: Failed, deliver: &dyn Fn(Heard)) {
        if !std::mem::replace(&mut self.lost, true) {
            deliver(Heard::Note(Note::Lost(self.input, failed)));
        }
    }

    fn device(&self) -> Result<(IMMDevice, String)> {
        match self.input {
            Input::System => default_device(eRender),
            Input::Microphone => microphone_device(self.wanted.as_deref()),
        }
    }

    fn connect(&mut self, deliver: &dyn Fn(Heard)) -> std::result::Result<Opened, Failed> {
        if self.input == Input::System && !self.whole_system {
            match process_loopback()
                .and_then(|client| start(client, None, AUDCLNT_STREAMFLAGS_LOOPBACK))
            {
                Ok(opened) => return Ok(opened),
                Err(e) => {
                    self.whole_system = true;
                    deliver(Heard::Note(Note::WithIsland(e.into())));
                }
            }
        }
        let (device, id) = self.device()?;
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
            | match self.input {
                Input::System => AUDCLNT_STREAMFLAGS_LOOPBACK,
                Input::Microphone => 0,
            };
        Ok(start(client, Some(id), flags)?)
    }

    fn read(&mut self, deliver: &dyn Fn(Heard)) {
        let Some(open) = &self.open else {
            return;
        };
        if let Err(e) = drain(self.input, &open.capture, deliver) {
            // Usually the device was unplugged or changed (AUDCLNT_E_DEVICE_INVALIDATED).
            self.open = None;
            self.next_try = Instant::now();
            self.lose(e.into(), deliver);
        }
    }
}

pub(super) fn drain(
    input: Input,
    capture: &IAudioCaptureClient,
    deliver: &dyn Fn(Heard),
) -> Result<()> {
    loop {
        if unsafe { capture.GetNextPacketSize()? } == 0 {
            return Ok(());
        }
        let mut data = std::ptr::null_mut();
        let (mut frames, mut flags, mut at) = (0u32, 0u32, 0u64);
        unsafe {
            capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut at))?;
        }
        let count = frames as usize * CHANNELS;
        let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
        let samples = if silent || data.is_null() {
            vec![0; count]
        } else {
            // SAFETY: Windows hands `frames` frames in the format we asked for (16 bit, stereo),
            // valid until ReleaseBuffer.
            unsafe { std::slice::from_raw_parts(data as *const i16, count) }.to_vec()
        };
        unsafe { capture.ReleaseBuffer(frames)? };
        deliver(Heard::Packet { input, at, samples });
    }
}

fn format() -> WAVEFORMATEX {
    let block = (CHANNELS * 2) as u16;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: CHANNELS as u16,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * u32::from(block),
        nBlockAlign: block,
        wBitsPerSample: 16,
        cbSize: 0,
    }
}

fn initialize(client: &IAudioClient, flags: u32) -> Result<()> {
    let format = format();
    unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER, 0, &format, None) }
}

pub(super) fn start(client: IAudioClient, device: Option<String>, flags: u32) -> Result<Opened> {
    initialize(&client, flags | AUDCLNT_STREAMFLAGS_EVENTCALLBACK)?;
    let event = unsafe { CreateEventW(None, false, false, None)? };
    let opened = |client: &IAudioClient| -> Result<IAudioCaptureClient> {
        unsafe {
            client.SetEventHandle(event)?;
            let capture: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            Ok(capture)
        }
    };
    match opened(&client) {
        Ok(capture) => Ok(Opened {
            client,
            capture,
            event,
            device,
        }),
        Err(e) => {
            unsafe {
                let _ = CloseHandle(event);
            }
            Err(e)
        }
    }
}

pub(super) fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

pub(super) fn device_id(device: &IMMDevice) -> Result<String> {
    unsafe {
        let raw = device.GetId()?;
        let id = raw.to_string().unwrap_or_default();
        CoTaskMemFree(Some(raw.0 as *const _));
        Ok(id)
    }
}

pub(super) fn default_device(flow: EDataFlow) -> Result<(IMMDevice, String)> {
    let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(flow, eConsole)? };
    let id = device_id(&device)?;
    Ok((device, id))
}

pub(super) fn plugged(id: &str) -> Option<IMMDevice> {
    let device = unsafe { enumerator().ok()?.GetDevice(&HSTRING::from(id)).ok()? };
    let state = unsafe { device.GetState().ok()? };
    (state == DEVICE_STATE_ACTIVE).then_some(device)
}

/// The chosen microphone if it is plugged in, otherwise the Windows default.
fn microphone_device(wanted: Option<&str>) -> Result<(IMMDevice, String)> {
    match wanted.and_then(|id| plugged(id).map(|device| (device, id.to_string()))) {
        Some(found) => Ok(found),
        None => default_device(eCapture),
    }
}

/// Everything the computer plays except this process and its children (Windows 10 2004+).
fn process_loopback() -> Result<IAudioClient> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: unsafe { GetCurrentProcessId() },
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // Never dropped: dropping a PROPVARIANT clears it, and clearing a blob frees its data,
    // which here is `params` on the stack.
    let mut variant = std::mem::ManuallyDrop::new(PROPVARIANT::default());
    // SAFETY: a blob pointing at `params`, which outlives the call.
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB {
            cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: &params as *const _ as *mut u8,
        };
    }
    let (tx, rx) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(Mutex::new(Some(tx))).into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*variant),
            &handler,
        )?
    };
    rx.recv_timeout(ACTIVATION).map_err(|_| Error::empty())?;
    let mut result = HRESULT(0);
    let mut activated: Option<IUnknown> = None;
    unsafe { operation.GetActivateResult(&mut result, &mut activated)? };
    result.ok()?;
    activated.ok_or_else(Error::empty)?.cast()
}

/// Windows calls it from its own thread when the activation is done.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(Mutex<Option<Sender<()>>>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(&self, _: Ref<IActivateAudioInterfaceAsyncOperation>) -> Result<()> {
        if let Some(done) = self.0.lock().ok().and_then(|mut done| done.take()) {
            let _ = done.send(());
        }
        Ok(())
    }
}
