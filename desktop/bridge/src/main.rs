//! Native messaging host the browsers start: relays one request frame from stdin to the app's socket and the
//! app's reply frame back to stdout. It never parses JSON, so protocol changes never touch it.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use github_magic_core::{MAX_FRAME, socket_path};

fn main() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    // Contents/MacOS/github-magic-bridge inside the bundle; no bundle in development.
    let bundle = exe.ancestors().nth(3).filter(|p| p.extension() == Some("app".as_ref()));
    run(&mut io::stdin().lock(), &mut io::stdout().lock(), &socket_path(), bundle)
}

fn run(input: &mut impl Read, output: &mut impl Write, socket: &Path, bundle: Option<&Path>) -> io::Result<()> {
    let request = read_frame(input)?;
    let reply = match connect(socket, bundle) {
        Ok(mut app) => write_frame(&mut app, &request)
            .and_then(|()| read_frame(&mut app))
            .unwrap_or_else(|e| error_reply(None, &e)),
        Err(e) => error_reply(Some("Github Magic isn't running"), &e),
    };
    write_frame(output, &reply)
}

fn connect(socket: &Path, bundle: Option<&Path>) -> io::Result<UnixStream> {
    let first = UnixStream::connect(socket);
    let Some(bundle) = bundle.filter(|_| first.is_err()) else { return first };
    // Through LaunchServices, not as our child: browsers may kill the host's process tree once it has replied.
    let open = Command::new("/usr/bin/open").arg("-g").arg("-a").arg(bundle).status()?;
    if !open.success() {
        return first;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        sleep(Duration::from_millis(100));
        match UnixStream::connect(socket) {
            Err(_) if Instant::now() < deadline => {}
            result => return result,
        }
    }
}

fn error_reply(title: Option<&str>, e: &io::Error) -> Vec<u8> {
    let mut reply = serde_json::json!({ "ok": false, "error": format!("Couldn't reach Github Magic: {e}") });
    if let Some(title) = title {
        reply["title"] = title.into();
    }
    reply.to_string().into_bytes()
}

/// A 4-byte native-endian length, then that many bytes.
fn read_frame(from: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0; 4];
    from.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::other(format!("frame of {len} bytes is over the 1 MB limit")));
    }
    let mut frame = vec![0; len];
    from.read_exact(&mut frame)?;
    Ok(frame)
}

fn write_frame(to: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    to.write_all(&(frame.len() as u32).to_ne_bytes())?;
    to.write_all(frame)?;
    to.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_frame(&mut out, body).unwrap();
        out
    }

    fn temp_socket(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("gm-{}-{name}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn relays_request_and_reply_unchanged() {
        let socket = temp_socket("relay");
        let listener = UnixListener::bind(&socket).unwrap();
        let app = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let request = read_frame(&mut conn).unwrap();
            write_frame(&mut conn, br#"{"ok":true,"path":"/tmp/x"}"#).unwrap();
            request
        });

        let mut output = Vec::new();
        run(&mut &frame(br#"{"action":"detect","repo":"a/b"}"#)[..], &mut output, &socket, None).unwrap();

        assert_eq!(app.join().unwrap(), br#"{"action":"detect","repo":"a/b"}"#);
        assert_eq!(output, frame(br#"{"ok":true,"path":"/tmp/x"}"#));
    }

    #[test]
    fn replies_itself_when_the_app_is_not_running() {
        let mut output = Vec::new();
        run(&mut &frame(b"{}")[..], &mut output, &temp_socket("missing"), None).unwrap();

        let reply: serde_json::Value = serde_json::from_slice(&read_frame(&mut &output[..]).unwrap()).unwrap();
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["title"], "Github Magic isn't running");
        assert!(reply["error"].as_str().unwrap().starts_with("Couldn't reach Github Magic: "));
    }

    #[test]
    fn replies_an_error_when_the_app_hangs_up() {
        let socket = temp_socket("hangup");
        let listener = UnixListener::bind(&socket).unwrap();
        let app = thread::spawn(move || drop(listener.accept().unwrap()));

        let mut output = Vec::new();
        run(&mut &frame(b"{}")[..], &mut output, &socket, None).unwrap();
        app.join().unwrap();

        let reply: serde_json::Value = serde_json::from_slice(&read_frame(&mut &output[..]).unwrap()).unwrap();
        assert_eq!(reply["ok"], false);
        assert!(reply.get("title").is_none());
    }

    #[test]
    fn refuses_frames_over_1_mb() {
        let mut input = ((MAX_FRAME + 1) as u32).to_ne_bytes().to_vec();
        input.resize(4 + MAX_FRAME + 1, b' ');
        assert!(run(&mut &input[..], &mut Vec::new(), &temp_socket("big"), None).is_err());
        assert_eq!(read_frame(&mut &frame(&vec![b' '; MAX_FRAME])[..]).unwrap().len(), MAX_FRAME);
    }
}
