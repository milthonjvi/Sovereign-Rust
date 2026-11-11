//! Allocation-free PCI hierarchy enumeration.
//!
//! This module walks PCI-to-PCI bridges without changing bridge configuration.
//! It only reads the primary/secondary/subordinate bus-number register and
//! enumerates buses reachable from the caller-supplied root bus.

use super::config::{
    ConfigAccess, ConfigError, PciAddress, PciFunction, PciIdentity, MAX_DEVICES,
    MAX_FUNCTIONS,
};

const PCI_CLASS_BRIDGE: u8 = 0x06;
const PCI_SUBCLASS_PCI_TO_PCI: u8 = 0x04;
const PCI_BRIDGE_BUS_NUMBERS: u16 = 0x18;
const BUS_COUNT: usize = 256;

/// Result of a hierarchy scan. `devices_found` includes devices that could not
/// fit in the caller's output slice; `written` is the number actually stored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EnumerationReport {
    pub written: usize,
    pub devices_found: u32,
    pub buses_scanned: u16,
    pub truncated: bool,
}

/// Enumerate the root bus and every valid secondary bus reachable through
/// PCI-to-PCI bridges.
///
/// The caller owns the output buffer; no allocation or configuration-space
/// writes are performed. A malformed bridge bus-number tuple is ignored rather
/// than followed. Each bus is queued at most once, preventing bridge cycles.
/// Errors from configuration-space reads are returned immediately.
pub fn scan_hierarchy<A: ConfigAccess>(
    io: &A,
    segment: u16,
    root_bus: u8,
    out: &mut [PciFunction],
) -> Result<EnumerationReport, ConfigError> {
    let mut queue = [0u8; BUS_COUNT];
    let mut queue_head = 0usize;
    let mut queue_tail = 0usize;
    let mut visited = [0u64; 4];
    let mut report = EnumerationReport::default();

    enqueue_bus(root_bus, &mut queue, &mut queue_tail, &mut visited);

    while queue_head < queue_tail {
        let bus = queue[queue_head];
        queue_head += 1;
        report.buses_scanned = report.buses_scanned.saturating_add(1);

        for device in 0..MAX_DEVICES {
            let first_address = PciAddress {
                segment,
                bus,
                device,
                function: 0,
            };
            let Some(first_identity) = PciIdentity::read(io, first_address)? else {
                continue;
            };

            process_function(
                io,
                PciFunction { address: first_address, identity: first_identity },
                &mut report,
                out,
                &mut queue,
                &mut queue_tail,
                &mut visited,
            )?;

            let function_limit = if first_identity.is_multifunction() {
                MAX_FUNCTIONS
            } else {
                1
            };
            for function in 1..function_limit {
                let address = PciAddress { function, ..first_address };
                let Some(identity) = PciIdentity::read(io, address)? else {
                    continue;
                };
                process_function(
                    io,
                    PciFunction { address, identity },
                    &mut report,
                    out,
                    &mut queue,
                    &mut queue_tail,
                    &mut visited,
                )?;
            }
        }
    }

    Ok(report)
}

fn process_function<A: ConfigAccess>(
    io: &A,
    function: PciFunction,
    report: &mut EnumerationReport,
    out: &mut [PciFunction],
    queue: &mut [u8; BUS_COUNT],
    queue_tail: &mut usize,
    visited: &mut [u64; 4],
) -> Result<(), ConfigError> {
    report.devices_found = report.devices_found.saturating_add(1);
    if report.written < out.len() {
        out[report.written] = function;
        report.written += 1;
    } else {
        report.truncated = true;
    }

    // Only type-1 PCI-to-PCI bridges expose the bus-number tuple at 0x18.
    if function.identity.header_type & 0x7f != 1
        || function.identity.class_code != PCI_CLASS_BRIDGE
        || function.identity.subclass != PCI_SUBCLASS_PCI_TO_PCI
    {
        return Ok(());
    }

    let buses = io.read_u32(function.address, PCI_BRIDGE_BUS_NUMBERS)?;
    let primary = buses as u8;
    let secondary = (buses >> 8) as u8;
    let subordinate = (buses >> 16) as u8;

    // Do not follow unconfigured, self-referential, or internally inconsistent
    // bridge ranges. The primary bus must identify the bus containing the
    // bridge; secondary must be a distinct, nonzero bus within the range.
    if primary != function.address.bus
        || secondary == 0
        || secondary == primary
        || secondary > subordinate
    {
        return Ok(());
    }

    enqueue_bus(secondary, queue, queue_tail, visited);
    Ok(())
}

fn enqueue_bus(
    bus: u8,
    queue: &mut [u8; BUS_COUNT],
    queue_tail: &mut usize,
    visited: &mut [u64; 4],
) {
    let word = (bus as usize) / 64;
    let bit = 1u64 << (bus as usize % 64);
    if visited[word] & bit != 0 {
        return;
    }
    visited[word] |= bit;
    // At most 256 distinct bus numbers exist, and buses are marked visited
    // before enqueueing, so this bound cannot be exceeded.
    if *queue_tail < queue.len() {
        queue[*queue_tail] = bus;
        *queue_tail += 1;
    }
}
