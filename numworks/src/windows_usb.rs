//! Windows host importer using an installed usbip-win2 client.

use std::{
    io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub(crate) const SETUP_URL: &str = "https://github.com/vadimgrn/usbip-win2/releases";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(crate) struct LocalUsb {
    stop: Option<Sender<()>>,
    pub updates: Receiver<Update>,
    pub port: Option<u16>,
}

pub(crate) enum Update {
    Attached(u16),
    Finished(Result<(), String>),
}

impl LocalUsb {
    pub fn start(server_port: u16) -> Result<Self, String> {
        let client = ["ProgramW6432", "ProgramFiles"]
            .into_iter()
            .filter_map(std::env::var_os)
            .filter(|directory| Path::new(directory).is_absolute())
            .map(|directory| PathBuf::from(directory).join(r"USBip\usbip.exe"))
            .find(|path| path.is_file())
            .ok_or("Install the signed usbip-win2 Windows importer to connect locally.")?;
        let (stop, shutdown) = mpsc::channel();
        let (send, updates) = mpsc::channel();
        thread::Builder::new()
            .name("N120 Windows USB importer".into())
            .spawn(move || {
                let result = connect(&client, server_port, &shutdown, &send);
                let _ = send.send(Update::Finished(result));
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop: Some(stop),
            updates,
            port: None,
        })
    }

    pub fn disconnect(&mut self) {
        // Closing the sender also lets the worker clean up on unexpected UI exit.
        self.stop = None;
    }

    pub fn disconnecting(&self) -> bool {
        self.stop.is_none()
    }
}

fn connect(
    client: &Path,
    server_port: u16,
    stop: &Receiver<()>,
    updates: &Sender<Update>,
) -> Result<(), String> {
    run(client, &["port"], Duration::from_secs(3)).map_err(|error| {
        format!("The Windows USB/IP driver is unavailable to this process. {error}")
    })?;
    if matches!(stop.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
        return Ok(());
    }
    let uri = format!("usbip://127.0.0.1:{server_port}/1-1");
    let server_port = server_port.to_string();
    let attached = run(
        client,
        &["-t", &server_port, "attach", "-r", "127.0.0.1", "-b", "1-1"],
        Duration::from_secs(15),
    );
    let port_list = run(client, &["port"], Duration::from_secs(3));
    let port = match (attached, port_list) {
        (Ok(_), Ok(list)) => find_port(&list, &uri)
            .ok_or("Windows did not report an imported device at the emulator's address.")?,
        (Err(error), Ok(list)) => {
            if let Some(port) = find_port(&list, &uri) {
                detach(client, port)?;
            }
            return Err(format!("Windows USB/IP attachment failed. {error}"));
        }
        (_, Err(error)) => {
            return Err(format!(
                "Could not query the Windows USB/IP driver. {error}"
            ));
        }
    };
    let _ = updates.send(Update::Attached(port));
    let _ = stop.recv();
    let list = run(client, &["port"], Duration::from_secs(3))?;
    if find_port(&list, &uri) == Some(port) {
        detach(client, port)?;
    }
    Ok(())
}

fn detach(client: &Path, port: u16) -> Result<(), String> {
    run(
        client,
        &["detach", "-p", &port.to_string()],
        Duration::from_secs(3),
    )
    .map(drop)
    .map_err(|error| format!("Could not detach Windows USB/IP port {port}. {error}"))
}

fn find_port(list: &str, uri: &str) -> Option<u16> {
    let mut port = None;
    for line in list.lines().map(str::trim) {
        if let Some((number, _)) = line
            .strip_prefix("Port ")
            .and_then(|line| line.split_once(':'))
        {
            port = number.trim().parse().ok();
        } else if line.strip_prefix("-> ") == Some(uri) {
            return port;
        }
    }
    None
}

fn run(client: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let poll = || -> io::Result<_> {
        let mut child = Command::new(client)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let expires = Instant::now() + timeout;
        while child.try_wait()?.is_none() {
            if Instant::now() >= expires {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "usbip.exe timed out",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        child.wait_with_output()
    };
    let output = poll().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let message = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "usbip.exe exited with {}. {}",
            output.status,
            message.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_the_emulators_imported_port() {
        let list = "Port 01: device in use\r\n    -> usbip://127.0.0.1:3240/1-1\r\n\
                    Port 02: device in use\r\n    -> usbip://127.0.0.1:65002/1-1\r\n";
        assert_eq!(find_port(list, "usbip://127.0.0.1:65002/1-1"), Some(2));
        assert_eq!(find_port(list, "usbip://127.0.0.1:65002/1"), None);
    }
}
