//! Loopback USB/IP server. The socket worker exchanges commands with the emulation thread.

use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    task::Poll,
    thread,
    time::{Duration, Instant},
};

use numworks::{Calculator, UsbControl, UsbPacket};

const VERSION: u16 = 0x0111;
const BUS_ID: &[u8] = b"1-1";
const HEADER: usize = 48;
const MAX_TRANSFER: usize = u16::MAX as usize;
const MAX_QUEUE: usize = 32;
const POLL: Duration = Duration::from_millis(10);
const TIMEOUT: Duration = Duration::from_secs(5);

const PIPE: i32 = -32;
const TIMEOUT_STATUS: i32 = -110;
const CANCELED: i32 = -104;

type TransferResult = Result<Vec<u8>, TransferError>;

#[derive(Debug)]
struct TransferError(i32, &'static str);

struct Request {
    transfer: Transfer,
    canceled: Arc<AtomicBool>,
    deadline: Option<Instant>,
    reply: Sender<TransferResult>,
}

enum Transfer {
    Control(UsbControl),
    Data {
        endpoint: u8,
        input: bool,
        length: usize,
        bytes: Vec<u8>,
        offset: usize,
        zero: bool,
    },
}

impl Transfer {
    fn poll(&mut self, calc: &mut Calculator) -> Poll<Result<Vec<u8>, &'static str>> {
        let Self::Data {
            endpoint,
            input,
            length,
            bytes,
            offset,
            zero,
        } = self
        else {
            let Self::Control(control) = self else {
                unreachable!()
            };
            return control.poll(calc);
        };
        let max = calc.usb_endpoint_packet_size(*endpoint, *input);
        if max == 0 {
            return Poll::Pending;
        }
        let packet = if *input {
            match calc.usb_endpoint_read(*endpoint) {
                UsbPacket::Ack(data) => {
                    let remaining = *length - bytes.len();
                    if data.len() > remaining {
                        return Poll::Ready(Err("USB IN packet exceeds the requested buffer"));
                    }
                    bytes.extend_from_slice(&data);
                    if bytes.len() == *length || data.len() < max {
                        return Poll::Ready(Ok(std::mem::take(bytes)));
                    }
                    UsbPacket::Ack(())
                }
                UsbPacket::Nak => UsbPacket::Nak,
                UsbPacket::Stall => UsbPacket::Stall,
                UsbPacket::Disconnected => UsbPacket::Disconnected,
            }
        } else {
            let end = (*offset + max).min(*length);
            let result = calc.usb_endpoint_write(*endpoint, &bytes[*offset..end]);
            if result == UsbPacket::Ack(()) {
                if end == *length && (!*zero || end == *offset || end - *offset < max) {
                    return Poll::Ready(Ok(Vec::new()));
                }
                *offset = end;
            }
            result
        };
        match packet {
            UsbPacket::Ack(()) | UsbPacket::Nak => Poll::Pending,
            UsbPacket::Stall => Poll::Ready(Err("guest stalled USB endpoint")),
            UsbPacket::Disconnected => Poll::Ready(Err("guest USB endpoint disconnected")),
        }
    }

    fn cancel(&self, calc: &mut Calculator) {
        if matches!(self, Self::Control(_)) {
            calc.usb_cancel_control();
        }
    }
}

enum Command {
    Attach,
    Detach,
    Control(Request),
}

#[derive(Default)]
struct Status {
    imported: bool,
    error: Option<String>,
}

pub struct UsbIp {
    pub port: u16,
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    listener: Option<thread::JoinHandle<()>>,
    epoch: Arc<AtomicU64>,
    status: Arc<Mutex<Status>>,
    receive: Receiver<(u64, Command)>,
    requests: Vec<Request>,
}

impl UsbIp {
    pub fn start(port: u16) -> io::Result<Self> {
        Self::listen(Ipv4Addr::LOCALHOST, port)
    }

    pub fn listen(interface: Ipv4Addr, port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind((interface, port))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(0));
        let status = Arc::new(Mutex::new(Status::default()));
        let (send, receive) = mpsc::channel();
        let worker = Worker {
            stop: stop.clone(),
            epoch: epoch.clone(),
            accepted_epoch: 0,
            status: status.clone(),
            send,
            claimed: Arc::new(AtomicBool::new(false)),
            trace: std::env::var_os("N120_USBIP_TRACE").is_some(),
        };
        let listener = thread::Builder::new()
            .name("N120 USB/IP".into())
            .spawn(move || worker.serve(listener))?;
        Ok(Self {
            port: address.port(),
            address,
            stop,
            listener: Some(listener),
            epoch,
            status,
            receive,
            requests: Vec::new(),
        })
    }

    pub fn status(&self) -> (bool, Option<String>) {
        let status = self.status.lock().unwrap();
        (status.imported, status.error.clone())
    }

    /// Drop the old virtual cable after a board reset and keep the listener open.
    pub fn disconnect_for_reset(&mut self, calc: &mut Calculator) {
        let mut status = self.status.lock().unwrap();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        *status = Status::default();
        drop(status);
        self.requests.clear();
        calc.usb_host_detach();
    }

    /// Call between CPU batches. This method never waits for the socket thread.
    pub fn service(&mut self, calc: &mut Calculator) {
        let epoch = self.epoch.load(Ordering::Acquire);
        while let Ok((accepted_epoch, command)) = self.receive.try_recv() {
            if accepted_epoch != epoch {
                continue;
            }
            match command {
                Command::Attach => calc.usb_host_attach(),
                Command::Detach => {
                    calc.usb_host_detach();
                    self.requests.clear();
                }
                Command::Control(request) => self.requests.push(request),
            }
        }
        self.requests.retain_mut(|request| {
            let result = if request.canceled.load(Ordering::Relaxed)
                || request
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                request.transfer.cancel(calc);
                Err(TransferError(
                    TIMEOUT_STATUS,
                    "USB transfer timed out or was canceled",
                ))
            } else if !calc.engine().is_running() {
                request.transfer.cancel(calc);
                Err(TransferError(
                    PIPE,
                    "resume the calculator before requesting USB transfers",
                ))
            } else {
                match request.transfer.poll(calc) {
                    Poll::Ready(result) => result.map_err(|error| TransferError(PIPE, error)),
                    Poll::Pending => return true,
                }
            };
            let _ = request.reply.send(result);
            false
        });
    }
}

impl Drop for UsbIp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(listener) = self.listener.take() {
            listener.thread().unpark();
            let _ = listener.join();
        }
    }
}

#[derive(Clone)]
struct Worker {
    stop: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    accepted_epoch: u64,
    status: Arc<Mutex<Status>>,
    send: Sender<(u64, Command)>,
    claimed: Arc<AtomicBool>,
    trace: bool,
}

impl Worker {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn disconnected(&self) -> bool {
        self.stopped() || self.epoch.load(Ordering::Acquire) != self.accepted_epoch
    }

    fn send(&self, command: Command) -> Result<(), mpsc::SendError<(u64, Command)>> {
        self.send.send((self.accepted_epoch, command))
    }

    fn serve(&self, listener: TcpListener) {
        let mut clients: Vec<thread::JoinHandle<()>> = Vec::new();
        while !self.stopped() {
            clients.retain(|client| !client.is_finished());
            match listener.accept() {
                Ok((mut socket, _)) if clients.len() < 8 => {
                    let mut worker = self.clone();
                    worker.accepted_epoch = self.epoch.load(Ordering::Acquire);
                    match thread::Builder::new()
                        .name("N120 USB/IP client".into())
                        .spawn(move || {
                            if let Err(error) = worker.connection(&mut socket)
                                && !matches!(
                                    error.kind(),
                                    io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                                )
                            {
                                let mut status = worker.status.lock().unwrap();
                                if !worker.disconnected() {
                                    status.error = Some(error.to_string());
                                }
                            }
                        }) {
                        Ok(client) => clients.push(client),
                        Err(error) => self.status.lock().unwrap().error = Some(error.to_string()),
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::park_timeout(POLL)
                }
                Err(error) => {
                    self.status.lock().unwrap().error = Some(error.to_string());
                    break;
                }
            }
        }
    }

    fn connection(&self, socket: &mut TcpStream) -> io::Result<()> {
        socket.set_nodelay(true)?;
        socket.set_read_timeout(Some(POLL))?;
        socket.set_write_timeout(Some(Duration::from_secs(1)))?;
        let mut header = [0; 8];
        self.read_management(socket, &mut header)?;
        let op = u16::from_be_bytes(header[2..4].try_into().unwrap());
        if u16::from_be_bytes(header[..2].try_into().unwrap()) != VERSION
            || !matches!(op, 0x8003 | 0x8005)
            || word(&header, 4) != 0
        {
            return Err(invalid("invalid USB/IP management header"));
        }
        let listing = op == 0x8005;
        if self.trace {
            eprintln!(
                "USB/IP {} from {:?}",
                if listing { "DEVLIST" } else { "IMPORT" },
                socket.peer_addr()
            );
        }
        if !listing {
            let mut bus = [0; 32];
            self.read_management(socket, &mut bus)?;
            if &bus[..BUS_ID.len()] != BUS_ID || bus[BUS_ID.len()..].iter().any(|&b| b != 0) {
                return socket.write_all(&management(op, 1));
            }
        }
        if self
            .claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            socket.write_all(&management(op, u32::from(!listing)))?;
            if listing {
                socket.write_all(&0u32.to_be_bytes())?;
            }
            return Ok(());
        }
        let result = self.export(socket, op, listing);
        {
            let mut status = self.status.lock().unwrap();
            if !self.disconnected() {
                status.imported = false;
            }
        }
        let _ = self.send(Command::Detach);
        self.claimed.store(false, Ordering::Release);
        result
    }

    fn export(&self, socket: &mut TcpStream, op: u16, listing: bool) -> io::Result<()> {
        self.send(Command::Attach)
            .map_err(|_| invalid("emulator closed"))?;
        let info = match self.device_info().and_then(|info| {
            if !listing {
                // The importing virtual host controller owns its local address.
                // Assign an address on the emulated bus before forwarding its URBs.
                self.control([0, 5, 1, 0, 0, 0, 0, 0])?;
            }
            Ok(info)
        }) {
            Ok(info) => info,
            Err(error) => {
                socket.write_all(&management(op, 1))?;
                return Err(error);
            }
        };
        socket.write_all(&management(op, 0))?;
        if listing {
            socket.write_all(&1u32.to_be_bytes())?;
        }
        socket.write_all(&info.device)?;
        if listing {
            return socket.write_all(&info.interfaces);
        }
        {
            let mut status = self.status.lock().unwrap();
            if self.disconnected() {
                return Ok(());
            }
            status.imported = true;
            status.error = None;
        }
        if self.trace {
            eprintln!("USB/IP import accepted for bus 1-1");
        }
        self.urbs(socket)
    }

    fn read_management(&self, socket: &mut TcpStream, mut dest: &mut [u8]) -> io::Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        while !dest.is_empty() {
            if self.disconnected() || Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "USB/IP management request timed out",
                ));
            }
            match socket.read(dest) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(size) => dest = &mut dest[size..],
                Err(error) if retry(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn submit(&self, setup: [u8; 8], data: Vec<u8>) -> Result<Pending, TransferError> {
        let control = UsbControl::new(setup, data).map_err(|error| TransferError(PIPE, error))?;
        self.submit_transfer(Transfer::Control(control))
    }

    fn submit_transfer(&self, transfer: Transfer) -> Result<Pending, TransferError> {
        let canceled = Arc::new(AtomicBool::new(false));
        let (reply, receive) = mpsc::channel();
        let deadline = matches!(transfer, Transfer::Control(_)).then(|| Instant::now() + TIMEOUT);
        self.send(Command::Control(Request {
            transfer,
            canceled: canceled.clone(),
            reply,
            deadline,
        }))
        .map_err(|_| TransferError(PIPE, "emulator closed"))?;
        Ok(Pending {
            canceled,
            receive,
            deadline,
        })
    }

    fn descriptor(&self, kind: u8, length: u16) -> io::Result<Vec<u8>> {
        let [lo, hi] = length.to_le_bytes();
        self.control([0x80, 6, 0, kind, 0, 0, lo, hi])
    }

    fn control(&self, setup: [u8; 8]) -> io::Result<Vec<u8>> {
        let pending = self
            .submit(setup, Vec::new())
            .map_err(|error| invalid(error.1))?;
        while !self.disconnected() {
            if let Some(result) = pending.result() {
                return result
                    .map_err(|error| invalid(format!("USB/IP setup {setup:02x?}: {}", error.1)));
            }
            thread::sleep(POLL);
        }
        Err(invalid("USB/IP server stopped"))
    }

    fn device_info(&self) -> io::Result<DeviceInfo> {
        let device = self.descriptor(1, 18)?;
        let header = self.descriptor(2, 9)?;
        if header.len() != 9 || header[..2] != [9, 2] {
            return Err(invalid("invalid firmware USB configuration header"));
        }
        let length = u16::from_le_bytes([header[2], header[3]]);
        if length < 9 {
            return Err(invalid("invalid firmware USB configuration length"));
        }
        let config = self.descriptor(2, length)?;
        if config.len() != usize::from(length) {
            return Err(invalid("truncated firmware USB configuration"));
        }
        DeviceInfo::from_descriptors(&device, &config)
    }

    fn urbs(&self, stream: &mut TcpStream) -> io::Result<()> {
        let mut buffer = Vec::new();
        let mut chunk = [0; 8192];
        let mut queue: VecDeque<Urb> = VecDeque::new();
        let mut active: Vec<(Urb, Pending)> = Vec::new();
        while !self.disconnected() {
            for index in (0..active.len()).rev() {
                if let Some(result) = active[index].1.result() {
                    let (urb, _) = active.remove(index);
                    if self.trace {
                        let (endpoint, input) = urb.lane();
                        let direction = if word(&urb.header, 12) == 1 || input {
                            "IN"
                        } else {
                            "OUT"
                        };
                        let seq = word(&urb.header, 4);
                        if endpoint == 0 {
                            eprintln!(
                                "USB/IP #{seq} EP0 {direction} setup={:02x?}",
                                &urb.header[40..48]
                            );
                        }
                        let status = result.as_ref().map_or_else(|error| error.0, |_| 0);
                        eprintln!("USB/IP #{seq} EP{endpoint} {direction} status={status}");
                    }
                    if let Err(error) = &result {
                        let mut status = self.status.lock().unwrap();
                        if !self.disconnected() {
                            status.error = Some(error.1.into());
                        }
                    }
                    urb.reply(stream, result)?;
                }
            }
            for _ in 0..queue.len() {
                let mut urb = queue.pop_front().unwrap();
                if active
                    .iter()
                    .any(|(current, _)| current.lane() == urb.lane())
                {
                    queue.push_back(urb);
                    continue;
                }
                match urb
                    .transfer()
                    .and_then(|transfer| self.submit_transfer(transfer))
                {
                    Ok(wait) => active.push((urb, wait)),
                    Err(error) => urb.reply(stream, Err(error))?,
                }
            }
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(size) => buffer.extend_from_slice(&chunk[..size]),
                Err(error) if retry(&error) => continue,
                Err(error) => return Err(error),
            }
            while let Some(urb) = Urb::take(&mut buffer)? {
                let seq = word(&urb.header, 4);
                if word(&urb.header, 0) == 2 {
                    let target = word(&urb.header, 20);
                    let canceled = if let Some(index) = active
                        .iter()
                        .position(|(urb, _)| word(&urb.header, 4) == target)
                    {
                        active.remove(index);
                        true
                    } else if let Some(index) =
                        queue.iter().position(|urb| word(&urb.header, 4) == target)
                    {
                        queue.remove(index);
                        true
                    } else {
                        false
                    };
                    let mut reply = reply_header(4, seq);
                    put(&mut reply, 20, if canceled { CANCELED as u32 } else { 0 });
                    stream.write_all(&reply)?;
                } else {
                    if queue.len() + active.len() >= MAX_QUEUE
                        || queue.iter().any(|urb| word(&urb.header, 4) == seq)
                        || active.iter().any(|(urb, _)| word(&urb.header, 4) == seq)
                    {
                        return Err(invalid(
                            "USB/IP request queue is full or sequence is duplicated",
                        ));
                    }
                    queue.push_back(urb);
                }
            }
        }
        Ok(())
    }
}

struct Pending {
    canceled: Arc<AtomicBool>,
    receive: Receiver<TransferResult>,
    deadline: Option<Instant>,
}

impl Pending {
    fn result(&self) -> Option<TransferResult> {
        match self.receive.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(TransferError(PIPE, "emulator closed")))
            }
            Err(mpsc::TryRecvError::Empty)
                if self
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline) =>
            {
                Some(Err(TransferError(
                    TIMEOUT_STATUS,
                    "firmware USB request timed out",
                )))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Relaxed);
    }
}

struct Urb {
    header: [u8; HEADER],
    data: Vec<u8>,
}

impl Urb {
    fn take(buffer: &mut Vec<u8>) -> io::Result<Option<Self>> {
        if buffer.len() < HEADER {
            return Ok(None);
        }
        let header = &buffer[..HEADER];
        let command = word(header, 0);
        let direction = word(header, 12);
        let length = word(header, 24) as usize;
        if command != 2
            && (command != 1
                || direction > 1
                || length > MAX_TRANSFER
                || !matches!(word(header, 32), 0 | u32::MAX))
        {
            return Err(invalid("unsupported or oversized USB/IP request"));
        }
        let end = HEADER
            + if command == 1 && direction == 0 {
                length
            } else {
                0
            };
        if buffer.len() < end {
            return Ok(None);
        }
        let header = header.try_into().unwrap();
        let data = buffer[HEADER..end].to_vec();
        buffer.drain(..end);
        Ok(Some(Self { header, data }))
    }

    fn validate(&self) -> Result<(), TransferError> {
        let endpoint = word(&self.header, 16);
        if endpoint >= 16 {
            return Err(TransferError(PIPE, "invalid USB endpoint"));
        }
        if endpoint != 0 {
            return Ok(());
        }
        if u32::from(self.header[40] >> 7) != word(&self.header, 12)
            || u32::from(u16::from_le_bytes([self.header[46], self.header[47]]))
                != word(&self.header, 24)
        {
            return Err(TransferError(
                PIPE,
                "USB/IP setup length or direction mismatch",
            ));
        }
        Ok(())
    }

    fn lane(&self) -> (u32, bool) {
        let endpoint = word(&self.header, 16);
        (endpoint, endpoint != 0 && word(&self.header, 12) == 1)
    }

    fn transfer(&mut self) -> Result<Transfer, TransferError> {
        self.validate()?;
        let bytes = std::mem::take(&mut self.data);
        let (endpoint, input) = self.lane();
        if endpoint == 0 {
            let setup = self.header[40..48].try_into().unwrap();
            let control =
                UsbControl::new(setup, bytes).map_err(|error| TransferError(PIPE, error))?;
            return Ok(Transfer::Control(control));
        }
        Ok(Transfer::Data {
            endpoint: endpoint as u8,
            input,
            length: word(&self.header, 24) as usize,
            bytes,
            offset: 0,
            zero: word(&self.header, 20) & 0x40 != 0,
        })
    }

    fn reply(&self, stream: &mut TcpStream, result: TransferResult) -> io::Result<()> {
        let mut reply = reply_header(3, word(&self.header, 4));
        let input = word(&self.header, 12) == 1;
        match result {
            Ok(data) => {
                put(
                    &mut reply,
                    24,
                    if input {
                        data.len() as u32
                    } else {
                        word(&self.header, 24)
                    },
                );
                stream.write_all(&reply)?;
                if input {
                    stream.write_all(&data)?;
                }
            }
            Err(error) => {
                put(&mut reply, 20, error.0 as u32);
                stream.write_all(&reply)?;
            }
        }
        Ok(())
    }
}

struct DeviceInfo {
    device: [u8; 312],
    interfaces: Vec<u8>,
}

impl DeviceInfo {
    fn from_descriptors(device: &[u8], config: &[u8]) -> io::Result<Self> {
        if device.len() != 18 || device[..2] != [18, 1] || config.len() < 9 || config[..2] != [9, 2]
        {
            return Err(invalid("invalid firmware USB descriptors"));
        }
        let mut interfaces = Vec::new();
        let mut found = [false; 256];
        let mut remaining = config;
        while !remaining.is_empty() {
            let length = remaining[0] as usize;
            if length < 2 || length > remaining.len() {
                return Err(invalid("malformed firmware USB descriptor"));
            }
            if remaining[1] == 4 && length >= 9 && !found[remaining[2] as usize] {
                found[remaining[2] as usize] = true;
                interfaces.extend_from_slice(&[remaining[5], remaining[6], remaining[7], 0]);
            }
            remaining = &remaining[length..];
        }
        if interfaces.len() / 4 != usize::from(config[4]) {
            return Err(invalid("firmware USB interface count mismatch"));
        }
        let mut block = [0; 312];
        let path = b"/n120/virtual/1-1";
        block[..path.len()].copy_from_slice(path);
        block[256..256 + BUS_ID.len()].copy_from_slice(BUS_ID);
        put(&mut block, 288, 1);
        put(&mut block, 292, 1);
        put(&mut block, 296, 2);
        block[300..312].copy_from_slice(&[
            device[9], device[8], device[11], device[10], device[13], device[12], device[4],
            device[5], device[6], config[5], device[17], config[4],
        ]);
        Ok(Self {
            device: block,
            interfaces,
        })
    }
}

fn management(op: u16, status: u32) -> [u8; 8] {
    let mut reply = [0; 8];
    reply[..2].copy_from_slice(&VERSION.to_be_bytes());
    reply[2..4].copy_from_slice(&(op & 0x7fff).to_be_bytes());
    put(&mut reply, 4, status);
    reply
}

fn reply_header(command: u32, sequence: u32) -> [u8; HEADER] {
    let mut reply = [0; HEADER];
    put(&mut reply, 0, command);
    put(&mut reply, 4, sequence);
    reply
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn retry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_invalidates_queued_usb_commands() {
        let mut internal = Vec::new();
        internal.extend_from_slice(&0x2405_0000u32.to_le_bytes());
        internal.extend_from_slice(&0x0800_0009u32.to_le_bytes());
        internal.extend_from_slice(&0xe7feu16.to_le_bytes());
        let mut calc = Calculator::boot(&numworks::Firmware {
            internal,
            ..Default::default()
        })
        .unwrap();
        let (send, receive) = mpsc::channel();
        let mut server = UsbIp {
            port: 0,
            address: (Ipv4Addr::LOCALHOST, 0).into(),
            stop: Arc::new(AtomicBool::new(false)),
            listener: None,
            epoch: Arc::new(AtomicU64::new(0)),
            status: Arc::new(Mutex::new(Status::default())),
            receive,
            requests: Vec::new(),
        };
        send.send((0, Command::Attach)).unwrap();
        server.service(&mut calc);
        assert!(calc.usb_host_attached());
        calc.set_usb_power(numworks::UsbPower::Charging);

        send.send((0, Command::Attach)).unwrap();
        server.disconnect_for_reset(&mut calc);
        server.service(&mut calc);
        assert!(!calc.usb_host_attached());
        assert!(calc.charger_active());
        assert!(!server.status().0);

        send.send((1, Command::Attach)).unwrap();
        send.send((0, Command::Detach)).unwrap();
        server.service(&mut calc);
        assert!(calc.usb_host_attached());
    }

    #[test]
    fn usb_ip_reads_fragmented_out_request_without_consuming_next_header() {
        let mut header = reply_header(1, 37);
        put(&mut header, 24, 2);
        header[46] = 2;
        let mut buffer = header.to_vec();
        buffer.push(0x34);
        assert!(Urb::take(&mut buffer).unwrap().is_none());
        buffer.push(0xab);
        buffer.extend(reply_header(2, 38));
        let urb = Urb::take(&mut buffer).unwrap().unwrap();
        assert_eq!(word(&urb.header, 4), 37);
        assert_eq!(urb.data, [0x34, 0xab]);
        assert!(urb.validate().is_ok());
        assert_eq!(word(&Urb::take(&mut buffer).unwrap().unwrap().header, 0), 2);
        assert!(buffer.is_empty());
        put(&mut header, 24, 65536);
        assert!(Urb::take(&mut header.to_vec()).is_err());
    }

    #[test]
    fn usb_ip_device_record_uses_firmware_ids_and_unique_interfaces() {
        let device = [
            18, 1, 0, 2, 2, 0, 0, 64, 0x83, 4, 0x40, 0x57, 1, 0, 1, 2, 3, 1,
        ];
        let config = [
            9, 2, 27, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 0, 2, 2, 1, 0, 9, 4, 0, 1, 0, 2, 2, 1, 0,
        ];
        let info = DeviceInfo::from_descriptors(&device, &config).unwrap();
        assert_eq!(&info.device[300..304], &[4, 0x83, 0x57, 0x40]);
        assert_eq!(info.device[311], 1);
        assert_eq!(info.interfaces, [2, 2, 1, 0]);
        assert!(DeviceInfo::from_descriptors(&device, &config[..26]).is_err());
    }
}
