//! A GET over WinHTTP: the system's proxy and certificates, redirects followed (a GitHub release file is
//! served from another host). Blocking: callers run it on a thread of their own.

use crate::fault::{Cause, Failed};
use std::{ffi::c_void, io::Write, path::Path};
use windows::{
    Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetTimeouts,
    },
    core::{Error, HSTRING, PCWSTR, w},
};

struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> Result<Self, Error> {
        if raw.is_null() {
            Err(Error::from_thread())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

struct Url<'a> {
    secure: bool,
    host: &'a str,
    port: u16,
    path: &'a str,
}

fn parse(url: &str) -> Result<Url<'_>, Failed> {
    let bad = || Failed::because(Cause::Unknown, format!("not an address to fetch: {url}"));
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        // Only for trying an update against a server on this computer.
        (false, rest)
    } else {
        return Err(bad());
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().map_err(|_| bad())?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    if !secure && !matches!(host, "localhost" | "127.0.0.1") {
        return Err(bad());
    }
    if host.is_empty() {
        return Err(bad());
    }
    Ok(Url {
        secure,
        host,
        port,
        path,
    })
}

struct Response {
    request: Handle,
    /// 0 when the server did not say.
    length: u64,
    _connection: Handle,
    _session: Handle,
}

fn request(url: &str) -> Result<Response, Failed> {
    let url = parse(url)?;
    unsafe {
        let session = Handle::new(WinHttpOpen(
            w!("GreatIsland"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        // resolve, connect, send, receive (ms): a silent server must not hold the thread for ever.
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 20_000, 30_000)?;
        let connection = Handle::new(WinHttpConnect(
            session.0,
            &HSTRING::from(url.host),
            url.port,
            0,
        ))?;
        let flags = if url.secure {
            WINHTTP_FLAG_SECURE
        } else {
            Default::default()
        };
        let request = Handle::new(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            &HSTRING::from(url.path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())?;

        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )?;
        if status != 200 {
            let cause = if status == 404 {
                Cause::Missing
            } else {
                Cause::Unknown
            };
            return Err(Failed::because(cause, format!("HTTP {status}")));
        }
        let mut length = 0u32;
        let mut size = size_of::<u32>() as u32;
        let known = WinHttpQueryHeaders(
            request.0,
            windows::Win32::Networking::WinHttp::WINHTTP_QUERY_CONTENT_LENGTH
                | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut length as *mut u32 as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )
        .is_ok();
        Ok(Response {
            request,
            length: if known { u64::from(length) } else { 0 },
            _connection: connection,
            _session: session,
        })
    }
}

impl Response {
    /// Up to `buffer.len()` bytes; 0 = the end.
    fn read(&self, buffer: &mut [u8]) -> Result<usize, Failed> {
        let mut read = 0u32;
        unsafe {
            WinHttpReadData(
                self.request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )?;
        }
        Ok(read as usize)
    }
}

/// The whole body, for a small answer: more than `limit` bytes is refused.
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, Failed> {
    let response = request(url)?;
    let mut body = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            return Ok(body);
        }
        body.extend_from_slice(&buffer[..read]);
        if body.len() > limit {
            return Err(Failed::because(Cause::Unknown, "the answer is too long"));
        }
    }
}

/// Into a file; `progress(done, total)` after every piece (`total` 0 = not known).
pub fn download(
    url: &str,
    path: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), Failed> {
    let response = request(url)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::File::create(path)?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            file.flush()?;
            return Ok(());
        }
        file.write_all(&buffer[..read])?;
        done += read as u64;
        progress(done, response.length);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_split() {
        let url = parse("https://github.com/a/b/releases/latest/download/latest.json").expect("ok");
        assert!(url.secure);
        assert_eq!(url.host, "github.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.path, "/a/b/releases/latest/download/latest.json");
        let local = parse("http://127.0.0.1:8080/latest.json").expect("ok");
        assert_eq!((local.host, local.port, local.secure), ("127.0.0.1", 8080, false));
    }

    #[test]
    fn plain_http_is_for_this_computer_only() {
        assert!(parse("http://example.org/latest.json").is_err());
        assert!(parse("ftp://example.org/a").is_err());
        assert!(parse("https://").is_err());
        assert!(parse("latest.json").is_err());
    }
}
