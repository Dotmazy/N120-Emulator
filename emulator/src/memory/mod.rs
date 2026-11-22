use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    peripherals::{Device, Gpio, Rcc, StorageOperation, system},
    profiles::{CpuType, MemoryKind, PeripheralKind, Profile},
    snapshot::SnapshotError,
};

const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Width {
    Byte = 1,
    Half = 2,
    Word = 4,
}

impl Width {
    pub fn bytes(self) -> u32 {
        self as u32
    }

    pub(crate) fn mask(self) -> u32 {
        match self {
            Self::Byte => 0xff,
            Self::Half => 0xffff,
            Self::Word => u32::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Read = 1,
    Write = 2,
    Execute = 4,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryError(String);

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for MemoryError {}

pub struct Bus {
    mappings: Vec<Mapping>,
    fetch_mapping: Cell<Option<usize>>,
    data_mapping: Cell<Option<usize>>,
    storage: Vec<Storage>,
    devices: Vec<Box<dyn Device>>,
    optional_snapshot_devices: Vec<usize>,
    controlled_storage: Vec<Option<usize>>,
    system_cycles: Option<Arc<AtomicU64>>,
    system: Option<system::System>,
}

/// A shared view of modeled core cycles, including idle advances while asleep.
#[derive(Clone)]
pub struct InstructionClock(Arc<AtomicU64>);

impl InstructionClock {
    pub fn ticks(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Handle for board devices to pend external Cortex-M interrupt lines.
#[derive(Clone)]
pub struct InterruptLines(system::System);

impl InterruptLines {
    pub fn pend(&self, irq: u8) {
        self.0.pend_irq(irq);
    }
    pub fn event(&self) {
        self.0.event();
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Mapping {
    name: String,
    base: u32,
    size: u32,
    permissions: u8,
    target: Target,
}

impl Mapping {
    fn end(&self) -> u64 {
        u64::from(self.base) + u64::from(self.size)
    }

    fn offset(&self, address: u32, len: u64) -> Option<u32> {
        let offset = address.checked_sub(self.base)?;
        (offset < self.size && len <= u64::from(self.size - offset)).then_some(offset)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Target {
    Storage(usize),
    Device { index: usize, gate: Option<Gate> },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Gate {
    device: usize,
    offset: u32,
    mask: u32,
}

struct Storage {
    fill: u8,
    size: u32,
    controller: Option<usize>,
    pages: Vec<Option<Box<[u8; PAGE_SIZE]>>>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct BusSnapshot {
    mappings: Vec<Mapping>,
    pages: Vec<BTreeMap<u32, Vec<u8>>>,
    devices: Vec<Vec<u8>>,
    cycles: Option<u64>,
}

impl Storage {
    fn apply(&mut self, operation: StorageOperation) -> Result<(), MemoryError> {
        let (offset, len) = match &operation {
            StorageOperation::Program { offset, bytes } => (*offset, bytes.len() as u64),
            StorageOperation::Erase { offset, len } => (*offset, u64::from(*len)),
        };
        if u64::from(offset) + len > u64::from(self.size) {
            return Err(MemoryError(
                "Flash operation is outside the storage region".into(),
            ));
        }
        match operation {
            StorageOperation::Program { offset, mut bytes } => {
                for (index, byte) in bytes.iter_mut().enumerate() {
                    *byte &= self.read(offset + index as u32);
                }
                self.write(offset, &bytes);
            }
            StorageOperation::Erase { offset, len } => {
                let mut start = offset as usize;
                let end = start + len as usize;
                while start < end {
                    let within = start % PAGE_SIZE;
                    let count = (end - start).min(PAGE_SIZE - within);
                    let page = &mut self.pages[start / PAGE_SIZE];
                    if count == PAGE_SIZE {
                        *page = None;
                    } else if let Some(page) = page {
                        page[within..within + count].fill(self.fill);
                    }
                    start += count;
                }
            }
        }
        Ok(())
    }

    fn read(&self, offset: u32) -> u8 {
        let page = offset as usize / PAGE_SIZE;
        let within = offset as usize % PAGE_SIZE;
        self.pages[page]
            .as_ref()
            .map_or(self.fill, |data| data[within])
    }

    fn read_width(&self, offset: u32, width: Width) -> u32 {
        let start = offset as usize % PAGE_SIZE;
        let count = width.bytes() as usize;
        let end = start + count;
        if end <= PAGE_SIZE {
            let Some(page) = &self.pages[offset as usize / PAGE_SIZE] else {
                return u32::from_le_bytes([self.fill; 4]) & width.mask();
            };
            return match width {
                Width::Byte => u32::from(page[start]),
                Width::Half => u32::from(u16::from_le_bytes(page[start..end].try_into().unwrap())),
                Width::Word => u32::from_le_bytes(page[start..end].try_into().unwrap()),
            };
        }
        let mut bytes = [0; 4];
        for (i, byte) in bytes[..count].iter_mut().enumerate() {
            *byte = self.read(offset + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    fn write(&mut self, mut offset: u32, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let page = offset as usize / PAGE_SIZE;
            let within = offset as usize % PAGE_SIZE;
            let count = bytes.len().min(PAGE_SIZE - within);
            let data = self.pages[page].get_or_insert_with(|| Box::new([self.fill; PAGE_SIZE]));
            data[within..within + count].copy_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if !bytes.is_empty() {
                offset += count as u32;
            }
        }
    }
}

impl Bus {
    fn device_name(&self, index: usize) -> &str {
        self.mappings
            .iter()
            .find_map(|mapping| match mapping.target {
                Target::Device { index: mapped, .. } if mapped == index => {
                    Some(mapping.name.as_str())
                }
                _ => None,
            })
            .unwrap_or("unmapped device")
    }

    pub(crate) fn save_state(&self) -> Result<BusSnapshot, SnapshotError> {
        let devices = self
            .devices
            .iter()
            .enumerate()
            .map(|(index, device)| {
                device
                    .save_state()
                    .map_err(|error| SnapshotError(format!("{}: {error}", self.device_name(index))))
            })
            .collect::<Result<_, _>>()?;
        Ok(BusSnapshot {
            mappings: self.mappings.clone(),
            pages: self
                .storage
                .iter()
                .map(|storage| {
                    storage
                        .pages
                        .iter()
                        .enumerate()
                        .filter_map(|(index, page)| Some((index as u32, page.as_ref()?.to_vec())))
                        .collect()
                })
                .collect(),
            devices,
            cycles: self.instruction_clock().map(|clock| clock.ticks()),
        })
    }

    pub(crate) fn restore_state(&mut self, saved: BusSnapshot) -> Result<(), SnapshotError> {
        let device_count = saved.devices.len();
        let older_map = device_count < self.devices.len()
            && (device_count..self.devices.len()).all(|index| self.optional_snapshot_devices.contains(&index))
            && self.mappings.iter().filter(|mapping| {
                !matches!(mapping.target, Target::Device { index, .. } if index >= device_count)
            }).eq(&saved.mappings);
        if !(older_map || saved.mappings == self.mappings && device_count == self.devices.len())
            || saved.pages.len() != self.storage.len()
            || saved.cycles.is_some() != self.system_cycles.is_some()
        {
            return Err(SnapshotError(
                "save-state memory map differs from the running machine".into(),
            ));
        }
        let pages = saved
            .pages
            .into_iter()
            .zip(&self.storage)
            .map(|(saved, region)| {
                let mut pages = vec![None; region.pages.len()];
                for (index, bytes) in saved {
                    let page = pages.get_mut(index as usize).ok_or_else(|| {
                        SnapshotError("save-state page is outside its region".into())
                    })?;
                    *page = Some(bytes.into_boxed_slice().try_into().map_err(|_| {
                        SnapshotError("invalid save-state memory page length".into())
                    })?);
                }
                Ok(pages)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let backup = self
            .devices
            .iter()
            .enumerate()
            .map(|(index, device)| {
                device
                    .save_state()
                    .map_err(|error| SnapshotError(format!("{}: {error}", self.device_name(index))))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, bytes) in saved.devices.iter().enumerate() {
            if let Err(error) = self.devices[index].restore_state(bytes) {
                for (prior, original) in self.devices[..index].iter_mut().zip(&backup) {
                    prior.restore_state(original)?;
                }
                return Err(SnapshotError(format!(
                    "{}: {error}",
                    self.device_name(index)
                )));
            }
        }
        for device in &mut self.devices[device_count..] {
            device.restore_missing_state();
        }
        for (region, pages) in self.storage.iter_mut().zip(pages) {
            region.pages = pages;
        }
        if let (Some(clock), Some(cycles)) = (&self.system_cycles, saved.cycles) {
            clock.store(cycles, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn new(profile: &Profile) -> Result<Self, MemoryError> {
        let mut bus = Self {
            mappings: Vec::new(),
            fetch_mapping: Cell::new(None),
            data_mapping: Cell::new(None),
            storage: Vec::new(),
            devices: Vec::new(),
            optional_snapshot_devices: Vec::new(),
            controlled_storage: Vec::new(),
            system_cycles: None,
            system: None,
        };
        let mut backing = HashMap::new();
        for region in &profile.memory {
            if region.alias_of.is_none() {
                let fill = if region.kind == MemoryKind::Ram {
                    0
                } else {
                    0xff
                };
                backing.insert(&region.name, (bus.storage.len(), region));
                bus.storage.push(Storage {
                    fill,
                    size: region.size,
                    controller: None,
                    pages: vec![None; (region.size as usize).div_ceil(PAGE_SIZE)],
                });
            }
        }
        for region in &profile.memory {
            let name = region.alias_of.as_ref().unwrap_or(&region.name);
            let (index, original) = backing.get(name).ok_or_else(|| {
                MemoryError(format!(
                    "{} refers to unknown backing region {name}",
                    region.name
                ))
            })?;
            let access_bits = permissions(&region.permissions)?;
            if region.size > original.size
                || region.kind != original.kind
                || access_bits & !permissions(&original.permissions)? != 0
            {
                return Err(MemoryError(format!(
                    "invalid backing region for {}",
                    region.name
                )));
            }
            bus.insert_mapping(Mapping {
                name: region.name.clone(),
                base: region.base,
                size: region.size,
                permissions: access_bits,
                target: Target::Storage(*index),
            })?;
        }

        let indices: HashMap<_, _> = profile
            .peripherals
            .iter()
            .enumerate()
            .map(|(index, device)| (&device.name, (index, device)))
            .collect();
        for (index, peripheral) in profile.peripherals.iter().enumerate() {
            let gate = if let Some(clock) = &peripheral.clock {
                let (device, rcc) = indices.get(&clock.rcc).ok_or_else(|| {
                    MemoryError(format!(
                        "{} refers to unknown RCC {}",
                        peripheral.name, clock.rcc
                    ))
                })?;
                let offset = Rcc::gpio_enable_offset(rcc.kind)
                    .filter(|offset| offset + 4 <= rcc.size)
                    .ok_or_else(|| {
                        MemoryError(format!("{} has no GPIO clock register", clock.rcc))
                    })?;
                if clock.bit >= 32 || peripheral.kind != PeripheralKind::Gpio {
                    return Err(MemoryError(format!(
                        "invalid clock gate for {}",
                        peripheral.name
                    )));
                }
                Some(Gate {
                    device: *device,
                    offset,
                    mask: 1 << clock.bit,
                })
            } else {
                None
            };
            let device: Box<dyn Device> = match peripheral.kind {
                PeripheralKind::Gpio => Box::new(Gpio::default()),
                kind => Box::new(Rcc::new(kind)),
            };
            let permissions = permissions(&peripheral.permissions)?;
            if permissions & Access::Execute as u8 != 0 {
                return Err(MemoryError(format!(
                    "{} cannot be executable",
                    peripheral.name
                )));
            }
            bus.insert_mapping(Mapping {
                name: peripheral.name.clone(),
                base: peripheral.base,
                size: peripheral.size,
                permissions,
                target: Target::Device { index, gate },
            })?;
            bus.devices.push(device);
        }
        Ok(bus)
    }

    /// Attach a caller-supplied, ungated read/write peripheral to an unused,
    /// word-aligned address range. The device participates in engine resets.
    pub fn map_device(
        &mut self,
        name: impl Into<String>,
        base: u32,
        size: u32,
        device: impl Device + 'static,
    ) -> Result<(), MemoryError> {
        self.map_backing(
            name.into(),
            base,
            size,
            Access::Read as u8 | Access::Write as u8,
            device,
        )
    }

    /// Append a peripheral whose reset state is used when loading older sessions.
    pub fn map_optional_snapshot_device(
        &mut self,
        name: impl Into<String>,
        base: u32,
        size: u32,
        device: impl Device + 'static,
    ) -> Result<(), MemoryError> {
        let index = self.devices.len();
        self.map_device(name, base, size, device)?;
        self.optional_snapshot_devices.push(index);
        Ok(())
    }

    /// Attach basic memory-mapped Cortex-M system registers, SysTick, and M7 DWT.
    pub fn map_cortex_system(&mut self, cpu: CpuType, vector_base: u32) -> Result<(), MemoryError> {
        let cycles = Arc::new(AtomicU64::new(0));
        let system = system::System::new(cpu, vector_base, cycles.clone());
        self.map_device(
            "Cortex-M system",
            system::BASE,
            system::SIZE,
            system.clone(),
        )?;
        if cpu == CpuType::CortexM7
            && let Err(error) = self.map_device(
                "Cortex-M DWT",
                0xe000_1000,
                0x1000,
                system::Dwt::new(system.clone()),
            )
        {
            let index = self
                .mappings
                .partition_point(|mapping| mapping.base < system::BASE);
            self.mappings.remove(index);
            self.devices.pop();
            return Err(error);
        }
        self.system_cycles = Some(cycles);
        self.system = Some(system);
        Ok(())
    }

    pub(crate) fn pending_exception(&self, basepri: u8, current: u16) -> Option<(u32, u16)> {
        self.system
            .as_ref()
            .and_then(|system| system.pending_exception(basepri, current))
    }

    pub(crate) fn take_reset_request(&self) -> bool {
        self.system
            .as_ref()
            .is_some_and(system::System::take_reset_request)
    }

    pub(crate) fn ack_exception(&self, number: u16) {
        if let Some(system) = &self.system {
            system.ack_exception(number);
        }
    }

    pub(crate) fn set_active_exception(&self, number: u16) {
        if let Some(system) = &self.system {
            system.set_active_exception(number);
        }
    }

    pub(crate) fn vector_base(&self) -> Option<u32> {
        self.system.as_ref().map(system::System::vector_base)
    }

    pub(crate) fn undefined_instruction_exception(
        &self,
        active: u16,
        primask: u32,
        basepri: u8,
    ) -> u16 {
        self.system.as_ref().map_or(3, |system| {
            system.undefined_instruction_exception(active, primask, basepri)
        })
    }

    pub(crate) fn advance_sleep(&mut self) {
        if let Some(system) = &mut self.system {
            system.advance_sleep();
        }
    }

    pub(crate) fn take_event(&self) -> bool {
        self.system.as_ref().is_some_and(system::System::take_event)
    }

    pub(crate) fn event(&self) {
        if let Some(system) = &self.system {
            system.event();
        }
    }

    pub fn interrupt_lines(&self) -> Option<InterruptLines> {
        self.system.clone().map(InterruptLines)
    }

    pub(crate) fn fpu_enabled(&self) -> bool {
        self.system
            .as_ref()
            .is_some_and(system::System::fpu_enabled)
    }

    pub(crate) fn automatic_fp_context(&self) -> bool {
        self.system
            .as_ref()
            .is_some_and(system::System::automatic_fp_context)
    }

    pub(crate) fn instruction_retired(&mut self) {
        if let Some(cycles) = &self.system_cycles {
            // Only the owning bus advances the clock. Shared handles can read
            // it while the core runs, so publish with a relaxed atomic store.
            let next = cycles.load(Ordering::Relaxed).wrapping_add(1);
            cycles.store(next, Ordering::Relaxed);
        }
    }

    pub fn instruction_clock(&self) -> Option<InstructionClock> {
        self.system_cycles.clone().map(InstructionClock)
    }

    /// Map read-only executable storage whose contents are supplied by a device.
    pub fn map_executable_storage(
        &mut self,
        name: impl Into<String>,
        base: u32,
        size: u32,
        device: impl Device + 'static,
    ) -> Result<(), MemoryError> {
        self.map_backing(
            name.into(),
            base,
            size,
            Access::Read as u8 | Access::Execute as u8,
            device,
        )
    }

    fn map_backing(
        &mut self,
        name: String,
        base: u32,
        size: u32,
        permissions: u8,
        device: impl Device + 'static,
    ) -> Result<(), MemoryError> {
        self.insert_mapping(Mapping {
            name,
            base,
            size,
            permissions,
            target: Target::Device {
                index: self.devices.len(),
                gate: None,
            },
        })?;
        self.devices.push(Box::new(device));
        Ok(())
    }

    /// Replace a device model while retaining its address, permissions, and clock gate.
    pub fn replace_device(
        &mut self,
        name: &str,
        device: impl Device + 'static,
    ) -> Result<(), MemoryError> {
        let target = self
            .mappings
            .iter()
            .find(|mapping| mapping.name == name)
            .map(|mapping| mapping.target);
        let Some(Target::Device { index, .. }) = target else {
            return Err(MemoryError(format!("no mapped device named {name}")));
        };
        self.devices[index] = Box::new(device);
        Ok(())
    }

    /// Route writes to a read-only Flash window through an existing MMIO controller.
    /// The backing bytes and mapping remain part of the normal bus snapshot.
    pub fn link_storage_controller(
        &mut self,
        region: &str,
        device: &str,
    ) -> Result<(), MemoryError> {
        let find = |name: &str| self.mappings.iter().find(|mapping| mapping.name == name);
        let Some(Target::Storage(storage)) = find(region).map(|mapping| mapping.target) else {
            return Err(MemoryError(format!("no storage region named {region}")));
        };
        let Some(Target::Device { index, .. }) = find(device).map(|mapping| mapping.target) else {
            return Err(MemoryError(format!("no mapped device named {device}")));
        };
        if self.storage[storage].controller.is_some()
            || self
                .controlled_storage
                .get(index)
                .is_some_and(Option::is_some)
        {
            return Err(MemoryError(
                "storage or controller is already linked".into(),
            ));
        }
        if self.controlled_storage.len() <= index {
            self.controlled_storage.resize(index + 1, None);
        }
        self.controlled_storage[index] = Some(storage);
        self.storage[storage].controller = Some(index);
        Ok(())
    }

    fn insert_mapping(&mut self, mapping: Mapping) -> Result<(), MemoryError> {
        if mapping.size == 0 || mapping.end() > (1u64 << 32) {
            return Err(MemoryError(format!("invalid bounds for {}", mapping.name)));
        }
        if mapping.name.is_empty()
            || self
                .mappings
                .iter()
                .any(|existing| existing.name == mapping.name)
        {
            return Err(MemoryError(format!(
                "empty or duplicate mapping name {:?}",
                mapping.name
            )));
        }
        if matches!(mapping.target, Target::Device { .. })
            && (!mapping.base.is_multiple_of(4) || !mapping.size.is_multiple_of(4))
        {
            return Err(MemoryError(format!(
                "{} must be word-aligned and word-sized",
                mapping.name
            )));
        }

        let index = self
            .mappings
            .partition_point(|existing| existing.base < mapping.base);
        let preceding = index.checked_sub(1).and_then(|i| self.mappings.get(i));
        let following = self.mappings.get(index);
        if preceding.is_some_and(|previous| previous.end() > u64::from(mapping.base))
            || following.is_some_and(|next| mapping.end() > u64::from(next.base))
        {
            return Err(MemoryError(format!(
                "{} overlaps an existing mapping",
                mapping.name
            )));
        }
        self.mappings.insert(index, mapping);
        Ok(())
    }

    fn locate_index(&self, address: u32, len: u64) -> Result<(usize, u32), MemoryError> {
        let index = self
            .mappings
            .partition_point(|region| region.base <= address);
        if let Some(index) = index.checked_sub(1)
            && let Some(offset) = self.mappings[index].offset(address, len)
        {
            return Ok((index, offset));
        }
        Err(MemoryError(format!(
            "unmapped or cross-region access at {address:#010x} ({len} bytes)"
        )))
    }

    fn locate(&self, address: u32, len: u64) -> Result<(&Mapping, u32), MemoryError> {
        let (index, offset) = self.locate_index(address, len)?;
        Ok((&self.mappings[index], offset))
    }

    fn access(
        &self,
        address: u32,
        width: Width,
        access: Access,
    ) -> Result<(Target, u32), MemoryError> {
        let len = u64::from(width.bytes());
        let (mapping, offset) = if let Some(index) = self.data_mapping.get()
            && let Some(mapping) = self.mappings.get(index)
            && let Some(offset) = mapping.offset(address, len)
        {
            (mapping, offset)
        } else {
            let (index, offset) = self.locate_index(address, len)?;
            self.data_mapping.set(Some(index));
            (&self.mappings[index], offset)
        };
        if mapping.permissions & access as u8 == 0
            && !(access == Access::Write
                && matches!(mapping.target, Target::Storage(index) if self.storage[index].controller.is_some()))
        {
            return Err(MemoryError(format!(
                "{access:?} denied in {} at {address:#010x}",
                mapping.name
            )));
        }
        if matches!(mapping.target, Target::Device { .. })
            && mapping.permissions & Access::Execute as u8 == 0
            && !address.is_multiple_of(width.bytes())
        {
            return Err(MemoryError(format!(
                "unaligned peripheral access at {address:#010x}"
            )));
        }
        Ok((mapping.target, offset))
    }

    fn enabled(&self, gate: Option<Gate>) -> bool {
        gate.is_none_or(|gate| self.devices[gate.device].read(gate.offset) & gate.mask != 0)
    }

    fn read_access(&self, address: u32, width: Width, access: Access) -> Result<u32, MemoryError> {
        let (target, offset) = self.access(address, width, access)?;
        Ok(self.read_target(target, offset, width))
    }

    fn read_target(&self, target: Target, offset: u32, width: Width) -> u32 {
        match target {
            Target::Storage(index) => self.storage[index].read_width(offset, width),
            Target::Device { index, gate } if self.enabled(gate) => {
                self.devices[index].read_access(offset, width) & width.mask()
            }
            Target::Device { .. } => 0,
        }
    }

    pub fn fetch(&self, address: u32) -> Result<u16, MemoryError> {
        if !address.is_multiple_of(2) {
            return Err(MemoryError(format!(
                "unaligned instruction fetch at {address:#010x}"
            )));
        }
        if let Some(index) = self.fetch_mapping.get()
            && let Some(mapping) = self.mappings.get(index)
            && mapping.permissions & Access::Execute as u8 != 0
            && let Some(offset) = mapping.offset(address, 2)
        {
            return Ok(self.read_target(mapping.target, offset, Width::Half) as u16);
        }
        let (index, offset) = self.locate_index(address, 2)?;
        let mapping = &self.mappings[index];
        if mapping.permissions & Access::Execute as u8 == 0 {
            return Err(MemoryError(format!(
                "Execute denied in {} at {address:#010x}",
                mapping.name
            )));
        }
        self.fetch_mapping.set(Some(index));
        Ok(self.read_target(mapping.target, offset, Width::Half) as u16)
    }

    pub fn read(&self, address: u32, width: Width) -> Result<u32, MemoryError> {
        self.read_access(address, width, Access::Read)
    }

    pub fn write(&mut self, address: u32, width: Width, value: u32) -> Result<(), MemoryError> {
        let (target, offset) = self.access(address, width, Access::Write)?;
        match target {
            Target::Storage(index) => {
                if let Some(controller) = self.storage[index].controller {
                    let device = &mut self.devices[controller];
                    device.write_storage(offset, width, value & width.mask());
                    if let Some(operation) = device.take_storage_operation() {
                        self.storage[index].apply(operation)?;
                    }
                } else {
                    self.storage[index]
                        .write(offset, &value.to_le_bytes()[..width.bytes() as usize]);
                }
            }
            Target::Device { index, gate } if self.enabled(gate) => {
                let device = &mut self.devices[index];
                device.write_access(offset, width, value & width.mask());
                if let Some(Some(storage)) = self.controlled_storage.get(index)
                    && let Some(operation) = device.take_storage_operation()
                {
                    self.storage[*storage].apply(operation)?;
                }
            }
            Target::Device { .. } => {}
        }
        Ok(())
    }

    pub(crate) fn check_write(&self, address: u32, width: Width) -> Result<(), MemoryError> {
        self.access(address, width, Access::Write).map(|_| ())
    }

    pub fn load(&mut self, address: u32, bytes: &[u8]) -> Result<(), MemoryError> {
        if bytes.is_empty() {
            return Err(MemoryError("firmware image is empty".into()));
        }
        let (mapping, offset) = self.locate(address, bytes.len() as u64)?;
        match mapping.target {
            Target::Storage(index) => {
                self.storage[index].write(offset, bytes);
                Ok(())
            }
            Target::Device { .. } => Err(MemoryError(
                "firmware must be loaded into a memory region".into(),
            )),
        }
    }

    pub(crate) fn reset_devices(&mut self, vector_base: u32) {
        for device in &mut self.devices {
            device.reset();
        }
        if let Some(system) = &self.system {
            system.set_reset_vector_base(vector_base);
        }
    }
}

fn permissions(text: &str) -> Result<u8, MemoryError> {
    let mut bits = 0;
    for permission in text.bytes() {
        let bit = match permission {
            b'r' => Access::Read as u8,
            b'w' => Access::Write as u8,
            b'x' => Access::Execute as u8,
            _ => return Err(MemoryError(format!("invalid memory permissions {text:?}"))),
        };
        if bits & bit != 0 {
            return Err(MemoryError(format!(
                "duplicate memory permission in {text:?}"
            )));
        }
        bits |= bit;
    }
    Ok(bits)
}
