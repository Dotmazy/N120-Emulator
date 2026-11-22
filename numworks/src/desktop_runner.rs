use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use numworks::Calculator;

use crate::usb_ip::UsbIp;

#[derive(Default)]
pub(crate) struct Machine {
    pub calc: Option<Calculator>,
    pub usb_ip: Option<UsbIp>,
    pub execution: Duration,
    pub usb_service: Duration,
}

#[derive(Default)]
struct Shared {
    machine: Mutex<Machine>,
    ui_waiting: AtomicBool,
    stop: AtomicBool,
}

pub(crate) struct Runner {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Runner {
    pub fn start(machine: Machine, benchmark: bool) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            machine: Mutex::new(machine),
            ..Shared::default()
        });
        let worker_state = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("n120-emulation".into())
            .spawn(move || worker_state.run(benchmark))?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub fn access(&self, update: impl FnOnce(&mut Machine)) {
        self.shared.ui_waiting.store(true, Ordering::Release);
        update(&mut self.shared.machine.lock().unwrap());
        self.shared.ui_waiting.store(false, Ordering::Release);
        self.worker.as_ref().unwrap().thread().unpark();
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let worker = self.worker.take().unwrap();
        worker.thread().unpark();
        let _ = worker.join();
    }
}

impl Shared {
    fn run(&self, benchmark: bool) {
        while !self.stop.load(Ordering::Acquire) {
            if self.ui_waiting.load(Ordering::Acquire) {
                thread::park();
                continue;
            }
            let delay = {
                let mut machine = self.machine.lock().unwrap();
                machine.advance(benchmark)
            };
            match delay {
                Some(delay) if delay.is_zero() => {}
                // On Windows std::thread::sleep uses a high-resolution timer
                // when available. Cap the sleep so UI changes can take effect
                // promptly, and group sub-millisecond waits into small bursts.
                Some(delay) => {
                    thread::sleep(delay.clamp(Duration::from_micros(150), Duration::from_millis(2)))
                }
                None => thread::park(),
            }
        }
    }
}

impl Machine {
    fn advance(&mut self, benchmark: bool) -> Option<Duration> {
        let calc = self.calc.as_mut()?;
        let deadline = Instant::now() + Duration::from_micros(500);
        let mut generation = calc.reset_generation();
        loop {
            if let Some(server) = &mut self.usb_ip {
                let start = benchmark.then(Instant::now);
                if generation == calc.reset_generation() {
                    server.service(calc);
                } else {
                    server.disconnect_for_reset(calc);
                    generation = calc.reset_generation();
                }
                if let Some(start) = start {
                    self.usb_service += start.elapsed();
                }
            }
            if !calc.engine().is_running() {
                return self.usb_ip.as_ref().map(|_| Duration::from_millis(2));
            }
            let start = benchmark.then(Instant::now);
            let delay = calc.tick();
            if let Some(start) = start {
                self.execution += start.elapsed();
            }
            if generation != calc.reset_generation() {
                continue;
            }
            if delay != Some(Duration::ZERO) || Instant::now() >= deadline {
                return if self.usb_ip.is_some() {
                    Some(delay.unwrap_or(Duration::MAX).min(Duration::from_millis(2)))
                } else {
                    delay
                };
            }
        }
    }
}
