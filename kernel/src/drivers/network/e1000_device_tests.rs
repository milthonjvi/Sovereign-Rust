use super::*;
    use crate::drivers::api::{DeviceClass, DeviceInfo};
    use super::super::e1000::{RXD_STATUS_DD, RXD_STATUS_EOP, TXD_STATUS_DD, REG_TDT, REG_RDT, REG_STATUS};
    use super::super::e1000_hw::E1000Registers;
    use std::vec::Vec;

    #[derive(Default)]
    struct MockRegisters {
        writes: Vec<(u32, u32)>,
        fail_register: Option<u32>,
    }

    unsafe impl E1000Registers for MockRegisters {
        type Error = ();

        fn read32(&mut self, offset: u32) -> Result<u32, Self::Error> {
            if offset == REG_STATUS {
                return Ok((1 << 1) | (2 << 6) | 1);
            }
            if offset == REG_RAL0 { return Ok(0x4433_2202); }
            if offset == REG_RAH0 { return Ok(0x8000_6655); }
            Ok(0)
        }

        fn write32(&mut self, offset: u32, value: u32) -> Result<(), Self::Error> {
            if self.fail_register == Some(offset) {
                return Err(());
            }
            self.writes.push((offset, value));
            Ok(())
        }

        fn dma_write_barrier(&mut self) {
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        }

        fn dma_read_barrier(&mut self) {
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        }
    }

    fn mock_dma_buffer<'a>(address: u64, bytes: &'a mut [u8]) -> DmaBuffer<'a> {
        DmaBuffer::new_for_test(address, bytes).unwrap()
    }

    fn test_queues() -> E1000QueueConfig {
        E1000QueueConfig::new(0x30000, 0x31000, 8, 8).unwrap()
    }

    fn supported_device() -> DeviceInfo {
        DeviceInfo {
            class: DeviceClass::Network,
            vendor_id: Some(PCI_VENDOR_INTEL),
            device_id: Some(super::super::e1000::DEVICE_82540EM),
            address: BusAddress::Pci { segment: 0, bus: 0, device: 3, function: 0 },
        }
    }

    #[test]
    fn shutdown_resets_dma_engines_and_prevents_software_reactivation() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        for (descriptor, buffer) in rx_descriptors.iter_mut().zip(rx_buffers.iter()) {
            descriptor.buffer_address = buffer.dma_address();
        }
        // SAFETY: the mock backend performs no DMA; these arrays model pinned mappings.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        device.registers.fail_register = Some(super::super::e1000::REG_CTRL);
        assert_eq!(device.shutdown(), Err(DriverError::HardwareFault));
        assert!(device.faulted);
        assert!(!device.ready);
        assert_eq!(device.interface.link, LinkState::Unknown);

        device.registers.fail_register = None;
        assert_eq!(device.shutdown(), Ok(()));
        assert!(device.stopped);
        assert!(!device.faulted);
        assert!(!device.ready);
        assert_eq!(device.interface.link, LinkState::Down);
        assert!(device.registers.writes.contains(&(super::super::e1000::REG_CTRL,
            super::super::e1000::CTRL_RST)));
        assert!(!device.registers.writes.contains(&(super::super::e1000::REG_RCTL, 0)));
        assert!(!device.registers.writes.contains(&(super::super::e1000::REG_TCTL, 0)));
        assert_eq!(device.initialize(&supported_device()), Err(DriverError::NotReady));
    }

    #[test]
    fn batch_transmit_copies_and_publishes_with_one_tail_write() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        for (descriptor, buffer) in rx_descriptors.iter_mut().zip(rx_buffers.iter()) {
            descriptor.buffer_address = buffer.dma_address();
        }
        // SAFETY: the mock register backend does not perform DMA; backing arrays model
        // the pinned rings and device-visible buffers required by the constructor.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        let first = [0x11u8; 64];
        let second = [0x22u8; 96];
        let oversized: [&[u8]; 8] = [&first; 8];
        let mut oversized_slots = [u16::MAX; 8];
        let mut oversized_submissions = [(0u64, 0usize); 8];
        assert_eq!(
            device.transmit_batch(&oversized, &mut oversized_slots, &mut oversized_submissions),
            Err(DriverError::ResourceUnavailable),
        );
        assert!(!device.registers.writes.iter().any(|(reg, _)| *reg == REG_TDT));

        let frames: [&[u8]; 2] = [&first, &second];
        let mut slots = [u16::MAX; 2];
        let mut submissions = [(0u64, 0usize); 2];
        assert_eq!(device.transmit_batch(&frames, &mut slots, &mut submissions), Ok(2));
        assert_eq!(slots, [0, 1]);
        assert_eq!(device.tx_buffers[0].as_slice()[..64], first);
        assert_eq!(device.tx_buffers[1].as_slice()[..96], second);
        assert_eq!(device.registers.writes.iter().filter(|(reg, _)| *reg == REG_TDT).count(), 1);
        assert!(device.registers.writes.contains(&(REG_TDT, 2)));

        assert_eq!(
            device.transmit_batch_with(2, &mut slots, &mut submissions, |index, buffer| {
                let length = 32 + index;
                buffer[..length].fill(0x44 + index as u8);
                Ok(length)
            }),
            Ok(2),
        );
        assert_eq!(slots, [2, 3]);
        assert_eq!(&device.tx_buffers[2].as_slice()[..32], &[0x44; 32]);
        assert_eq!(&device.tx_buffers[3].as_slice()[..33], &[0x45; 33]);
        assert!(device.registers.writes.contains(&(REG_TDT, 4)));

        let tdt_writes_before_rejected_batch = device.registers.writes
            .iter().filter(|(reg, _)| *reg == REG_TDT).count();
        assert_eq!(
            device.transmit_batch_with(2, &mut slots, &mut submissions, |index, buffer| {
                buffer[..8].fill(0xaa);
                if index == 0 {
                    Ok(8)
                } else {
                    Err(DriverError::InvalidConfiguration)
                }
            }),
            Err(DriverError::InvalidConfiguration),
        );
        assert_eq!(
            device.registers.writes.iter().filter(|(reg, _)| *reg == REG_TDT).count(),
            tdt_writes_before_rejected_batch,
        );
        assert_eq!(device.tx_descriptors[4].status, TXD_STATUS_DD);
        assert_eq!(device.tx_descriptors[5].status, TXD_STATUS_DD);

        assert_eq!(
            device.transmit_with(|buffer| {
                buffer[..16].fill(0x55);
                16
            }),
            Ok(()),
        );
        assert_eq!(&device.tx_buffers[4].as_slice()[..16], &[0x55; 16]);
        assert!(device.registers.writes.contains(&(REG_TDT, 5)));

        device.registers.fail_register = Some(REG_TDT);
        assert_eq!(device.transmit(&[0x33; 64]), Err(DriverError::HardwareFault));
        assert_eq!(device.tx_descriptors[5].status, TXD_STATUS_DD);
        assert!(!device.ready);
        assert!(device.faulted);
        assert_eq!(device.shutdown(), Ok(()));
    }

    #[test]
    fn receive_drains_fragmented_frame_before_delivering_next_packet() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        rx_storage[2][..64].fill(0x53);
        for (index, descriptor) in rx_descriptors.iter_mut().enumerate() {
            descriptor.buffer_address = 0x10000 + index as u64 * 2048;
            if index < 3 {
                descriptor.length = if index == 0 { 2048 } else { 64 };
                descriptor.status = RXD_STATUS_DD
                    | if index == 0 { 0 } else { RXD_STATUS_EOP };
            }
        }
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        // SAFETY: the mock backend performs no DMA; arrays model pinned mappings.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        let mut output = [0u8; 128];
        assert_eq!(device.receive(&mut output), Ok(Some(64)));
        assert_eq!(&output[..64], &[0x53; 64]);
        assert_eq!(
            device.registers.writes.iter().filter(|(reg, _)| *reg == REG_RDT).count(),
            3,
        );
        assert!(!device.dropping_rx_frame);
    }

    #[test]
    fn batch_receive_copies_frames_and_recycles_with_one_tail_write() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        rx_storage[0][..64].fill(0x31);
        rx_storage[1][..96].fill(0x42);
        rx_storage[2][..64].fill(0x53);
        for (index, descriptor) in rx_descriptors.iter_mut().enumerate() {
            descriptor.buffer_address = 0x10000 + index as u64 * 2048;
            if index < 3 {
                descriptor.length = match index { 0 => 64, 1 => 96, _ => 64 };
                descriptor.status = RXD_STATUS_DD | RXD_STATUS_EOP;
            }
        }
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        // SAFETY: the mock register backend does not perform DMA; backing arrays model
        // the pinned rings and device-visible buffers required by the constructor.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        let group = super::super::ethernet::MacAddress([0x33, 0x33, 0, 0, 0, 1]);
        let hash = E1000MulticastFilter::hash(group) as usize;
        assert_eq!(device.set_multicast_filter(&[group]), Ok(()));
        assert!(device.multicast_filter.accepts(group));
        assert_eq!(
            device.registers.writes.last(),
            Some(&(REG_MTA_BASE + (hash >> 5) as u32 * 4, 1u32 << (hash & 0x1f))),
        );
        let writes_before_invalid = device.registers.writes.len();
        assert_eq!(
            device.set_multicast_filter(&[super::super::ethernet::MacAddress([0x02, 0, 0, 0, 0, 1])]),
            Err(DriverError::InvalidConfiguration),
        );
        assert_eq!(device.registers.writes.len(), writes_before_invalid);
        assert_eq!(device.set_multicast_filter(&[]), Ok(()));
        assert!(device.multicast_filter.is_empty());

        let mut first = [0u8; 128];
        let mut second = [0u8; 128];
        let mut outputs: [&mut [u8]; 2] = [&mut first, &mut second];
        let mut completions = [(0u16, super::super::e1000_descriptor::RxCompletion {
            length: 0, end_of_packet: false, errors: 0,
        }); 2];
        let mut backup = [RxDescriptor::default(); 2];
        assert_eq!(device.receive_batch(&mut outputs, &mut completions, &mut backup), Ok(2));
        assert_eq!(&first[..64], &[0x31; 64]);
        assert_eq!(&second[..96], &[0x42; 96]);

        let mut observed = [0u8; 64];
        let mut observed_length = 0usize;
        assert_eq!(
            device.receive_with(|frame| {
                observed_length = frame.len();
                observed[..frame.len()].copy_from_slice(frame);
            }),
            Ok(1),
        );
        assert_eq!(observed_length, 64);
        assert_eq!(observed, [0x53; 64]);
        assert_eq!(device.registers.writes.iter().filter(|(reg, _)| *reg == REG_RDT).count(), 1);
        assert!(device.registers.writes.contains(&(REG_RDT, 1)));
    }
