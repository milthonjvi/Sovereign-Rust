use super::*;
    use std::vec::Vec;

    fn mock_dma_buffer<'a>(address: u64, bytes: &'a mut [u8]) -> DmaBuffer<'a> {
        DmaBuffer::new_for_test(address, bytes).unwrap()
    }


    #[derive(Default)]
    struct MockRegisters {
        writes: Vec<(u32, u32)>,
        fail_register: Option<u32>,
        read_barriers: usize,
        reset_stuck: bool,
        multicast_mac: bool,
    }

    unsafe impl E1000Registers for MockRegisters {
        type Error = ();

        fn read32(&mut self, offset: u32) -> Result<u32, Self::Error> {
            if offset == REG_CTRL {
                return Ok(if self.reset_stuck { CTRL_RST } else { 0 });
            }
            if offset == REG_RAL0 {
                return Ok(if self.multicast_mac { 0x4433_2211 } else { 0x4433_2202 });
            }
            if offset == REG_STATUS { return Ok(STATUS_LU | STATUS_FD | (2 << STATUS_SPEED_SHIFT)); }
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
        fn dma_write_barrier(&mut self) { core::sync::atomic::fence(core::sync::atomic::Ordering::Release); }
        fn dma_read_barrier(&mut self) {
            self.read_barriers += 1;
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        }
    }

    #[test]
    fn reset_masks_interrupts_and_waits_for_self_clearing_control_bit() {
        let mut regs = MockRegisters::default();
        assert_eq!(reset(&mut regs), Ok(()));
        assert!(regs.writes.contains(&(REG_IMC, u32::MAX)));
        assert!(regs.writes.contains(&(REG_CTRL, CTRL_RST)));
    }

    #[test]
    fn reset_reports_bounded_timeout_when_hardware_never_clears_reset() {
        let mut regs = MockRegisters { reset_stuck: true, ..MockRegisters::default() };
        assert_eq!(reset(&mut regs), Err(E1000ResetError::Timeout));
    }

    #[test]
    fn link_state_reports_speed_and_duplex() {
        let mut regs = MockRegisters::default();
        assert_eq!(read_link_state(&mut regs), Ok(super::super::device::LinkState::Up {
            speed_mbps: Some(1000), full_duplex: true,
        }));
    }

    #[test]
    fn queue_config_enforces_hardware_ring_length_alignment() {
        assert!(E1000QueueConfig::defaults(0x1000, 0x2000).is_ok());
        assert!(E1000QueueConfig::new(0x1000, 0x2000, 12, 32).is_err());
    }

    #[test]
    fn queue_config_rejects_overlapping_descriptor_rings() {
        assert_eq!(
            E1000QueueConfig::new(0x1000, 0x1040, 8, 8),
            Err(super::super::e1000::E1000Error::OverlappingDmaRanges),
        );
    }

    #[test]
    fn initialization_rejects_rx_buffers_overlapping_descriptor_rings() {
        let mut regs = MockRegisters::default();
        let queues = E1000QueueConfig::defaults(0x4000, 0x8000).unwrap();
        let mut rx = [RxDescriptor::default(); 32];
        let mut rx_storage = [[0u8; 2048]; 32];
        let rx_buffers: Vec<DmaBuffer<'_>> = rx_storage.iter_mut().enumerate()
            .map(|(index, bytes)| mock_dma_buffer(0x4000 + index as u64 * 2048, bytes))
            .collect();
        let mut tx = [TxDescriptor::default(); 32];
        // SAFETY: MockRegisters never performs DMA; synthetic ranges intentionally overlap.
        assert_eq!(
            unsafe { initialize(&mut regs, queues, &mut rx, &rx_buffers, &mut tx, true) },
            Err(E1000InitError::InvalidDescriptorBuffers),
        );
        assert!(regs.writes.is_empty());
    }

    #[test]
    fn initialization_programs_rings_and_enables_rx_tx() {
        let mut regs = MockRegisters::default();
        let queues = E1000QueueConfig::defaults(0x1_0000_1000, 0x1_0000_2000).unwrap();
        let mut rx = [RxDescriptor::default(); 32];
        let mut rx_storage = [[0u8; 2048]; 32];
        let rx_buffers: Vec<DmaBuffer<'_>> = rx_storage.iter_mut().enumerate()
            .map(|(index, bytes)| mock_dma_buffer(0x4000 + index as u64 * 2048, bytes))
            .collect();
        let mut tx = [TxDescriptor::default(); 32];
        // SAFETY: MockRegisters does not perform DMA; these buffers model the
        // pinned mappings supplied by the platform in a real initialization.
        let info = unsafe { initialize(&mut regs, queues, &mut rx, &rx_buffers, &mut tx, true) }.unwrap();
        assert_eq!(info.mac, MacAddress([0x02, 0x22, 0x33, 0x44, 0x55, 0x66]));
        assert_eq!(
            regs.writes.iter().filter(|(reg, _)| {
                *reg >= REG_MTA_BASE
                    && *reg < REG_MTA_BASE + E1000_MTA_REGISTER_COUNT as u32 * 4
            }).count(),
            E1000_MTA_REGISTER_COUNT,
        );
        assert!(regs.writes.iter().filter(|(reg, _)| {
            *reg >= REG_MTA_BASE
                && *reg < REG_MTA_BASE + E1000_MTA_REGISTER_COUNT as u32 * 4
        }).all(|(_, value)| *value == 0));

        assert_eq!(rx[0].buffer_address, 0x4000);
        assert_eq!(rx[0].status, 0);
        assert_eq!(tx[0].status, TXD_STATUS_DD);
        assert!(regs.writes.contains(&(REG_RDBAH, 1)));
        assert!(regs.writes.contains(&(REG_TDBAH, 1)));
        assert!(regs.writes.contains(&(REG_RDT, 31)));
        assert!(regs.writes.contains(&(REG_TDT, 0)));
        assert!(regs.writes.contains(&(REG_RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC)));
        assert!(!regs.writes.contains(&(REG_RCTL, 0)));
        assert!(regs.writes.iter().any(|(reg, value)| *reg == REG_TCTL && value & TCTL_EN != 0));
    }

    #[test]
    fn initialization_rejects_multicast_hardware_mac_before_enabling_queues() {
        let mut regs = MockRegisters { multicast_mac: true, ..MockRegisters::default() };
        let queues = E1000QueueConfig::defaults(0x1_0000_1000, 0x1_0000_2000).unwrap();
        let mut rx = [RxDescriptor::default(); 32];
        let mut rx_storage = [[0u8; 2048]; 32];
        let rx_buffers: Vec<DmaBuffer<'_>> = rx_storage.iter_mut().enumerate()
            .map(|(index, bytes)| mock_dma_buffer(0x4000 + index as u64 * 2048, bytes))
            .collect();
        let mut tx = [TxDescriptor::default(); 32];

        // SAFETY: MockRegisters does not perform DMA; the buffers model pinned mappings.
        let result = unsafe { initialize(&mut regs, queues, &mut rx, &rx_buffers, &mut tx, true) };
        assert!(matches!(result, Err(E1000InitError::InvalidMacAddress)));
        assert!(!regs.writes.iter().any(|(reg, value)| {
            (*reg == REG_RCTL && value & RCTL_EN != 0)
                || (*reg == REG_TCTL && value & TCTL_EN != 0)
        }));
    }

    #[test]
    fn tx_queue_tracks_in_flight_descriptors_and_reclaims_in_order() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(0)));
        assert_eq!(queue.submit(&mut regs, &mut descriptors, 0x8000, 128), Ok(0));
        assert_eq!(queue.reap(&mut regs, &descriptors), Ok(0));
        descriptors[0].status = TXD_STATUS_DD;
        assert_eq!(queue.reap_one(&mut regs, &descriptors), Ok(Some(0)));
        assert_eq!(queue.reap(&mut regs, &descriptors), Ok(0));
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(1)));
        assert_eq!(queue.submit(&mut regs, &mut descriptors, 0x9000, 256), Ok(1));
    }

    #[test]
    fn tx_next_available_slot_tracks_descriptor_ownership() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(0)));
        assert_eq!(queue.submit(&mut regs, &mut descriptors, 0x8000, 64), Ok(0));
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(1)));
        descriptors[1].status = 0;
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(None));
        descriptors[1].status = TXD_STATUS_DD;
        descriptors[0].status = TXD_STATUS_DD;
        assert_eq!(queue.reap_one(&mut regs, &descriptors), Ok(Some(0)));
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(1)));
    }

    #[test]
    fn tx_available_slots_returns_contiguous_batch_in_ring_order() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        let mut slots = [u16::MAX; 4];
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut slots), Ok(4));
        assert_eq!(slots, [0, 1, 2, 3]);

        assert_eq!(queue.submit_batch(&mut regs, &mut descriptors, &[(0x8000, 64), (0x9000, 64)]), Ok(2));
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut slots), Ok(2));
        assert_eq!(&slots[..2], &[2, 3]);
    }

    #[test]
    fn tx_reclaim_and_slot_discovery_use_one_dma_read_barrier() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        let mut initial_slots = [0u16; 2];
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut initial_slots), Ok(2));
        assert_eq!(queue.submit_batch(&mut regs, &mut descriptors, &[(0x8000, 64), (0x9000, 96)]), Ok(2));
        descriptors[0].status = TXD_STATUS_DD;
        descriptors[1].status = TXD_STATUS_DD;
        let barriers_before = regs.read_barriers;
        let mut slots = [u16::MAX; 4];
        assert_eq!(queue.reclaim_and_available_slots(&mut regs, &descriptors, &mut slots), Ok(4));
        assert_eq!(slots, [2, 3, 4, 5]);
        assert_eq!(queue.pending, 0);
        assert_eq!(regs.read_barriers, barriers_before + 1);
    }

    #[test]
    fn tx_batch_rolls_back_descriptor_ownership_when_tail_write_fails() {
        let mut regs = MockRegisters::default();
        regs.fail_register = Some(REG_TDT);
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        let mut free_slots = [0u16; 2];
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut free_slots), Ok(2));
        assert_eq!(queue.submit_batch(&mut regs, &mut descriptors, &[(0x8000, 64), (0x9000, 96)]),
            Err(E1000QueueError::Register(())));
        assert_eq!(descriptors[0].status & TXD_STATUS_DD, TXD_STATUS_DD);
        assert_eq!(descriptors[1].status & TXD_STATUS_DD, TXD_STATUS_DD);
        assert_eq!(queue.pending, 0);
        regs.fail_register = None;
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut free_slots), Ok(2));
        assert_eq!(queue.submit_batch(&mut regs, &mut descriptors, &[(0xa000, 64)]), Ok(1));
    }

    #[test]
    fn tx_batch_publishes_multiple_descriptors_with_one_tail_update() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        let mut free_slots = [0u16; 2];
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut free_slots), Ok(2));
        assert_eq!(queue.submit_batch(&mut regs, &mut descriptors, &[(0x8000, 128), (0x9000, 256)]), Ok(2));
        assert_eq!(regs.writes.iter().filter(|(register, _)| *register == REG_TDT).count(), 1);
        assert!(regs.writes.contains(&(REG_TDT, 2)));
        descriptors[0].status = TXD_STATUS_DD;
        descriptors[1].status = TXD_STATUS_DD;
        assert_eq!(queue.reap(&mut regs, &descriptors), Ok(2));
    }

    #[test]
    fn rx_queue_rearms_only_a_completed_descriptor() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [RxDescriptor::for_buffer(0xa000); 8];
        let mut queue = E1000RxQueue::new(8).unwrap();
        assert_eq!(queue.poll(&mut regs, &mut descriptors), Ok(None));
        descriptors[0].length = 96;
        descriptors[0].status = super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP;
        let (index, completion) = queue.poll(&mut regs, &descriptors).unwrap().unwrap();
        assert_eq!(index, 0);
        assert_eq!(completion.length, 96);
        assert_eq!(descriptors[0].buffer_address, 0xa000);
        assert_eq!(descriptors[0].status & super::super::e1000::RXD_STATUS_DD, super::super::e1000::RXD_STATUS_DD);
        assert!(queue.recycle(&mut regs, &mut descriptors, index).is_ok());
        assert_eq!(descriptors[0].status, 0);
        assert!(regs.writes.contains(&(REG_RDT, 0)));
    }

    #[test]
    fn rx_batch_recycles_contiguous_descriptors_with_one_tail_update() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [RxDescriptor::for_buffer(0xa000); 8];
        let mut queue = E1000RxQueue::new(8).unwrap();
        let mut completions = [(0u16, super::super::e1000_descriptor::RxCompletion {
            length: 0, end_of_packet: false, errors: 0,
        }); 2];
        descriptors[0].length = 64;
        descriptors[0].status = super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP;
        descriptors[1].length = 96;
        descriptors[1].status = super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP;
        assert_eq!(queue.poll_batch(&mut regs, &descriptors, &mut completions), Ok(2));
        let mut backup = [RxDescriptor::default(); 2];
        assert_eq!(queue.recycle_batch(&mut regs, &mut descriptors, &completions, &mut backup), Ok(2));
        assert_eq!(descriptors[0].status, 0);
        assert_eq!(descriptors[1].status, 0);
        assert_eq!(regs.writes.iter().filter(|(register, _)| *register == REG_RDT).count(), 1);
        assert!(regs.writes.contains(&(REG_RDT, 1)));
        assert_eq!(queue.poll(&mut regs, &descriptors), Ok(None));
    }

    #[test]
    fn rx_batch_restores_descriptors_when_tail_publication_fails() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [RxDescriptor::for_buffer(0xa000); 8];
        let mut queue = E1000RxQueue::new(8).unwrap();
        let mut completions = [(0u16, super::super::e1000_descriptor::RxCompletion {
            length: 64, end_of_packet: true, errors: 0,
        }); 1];
        descriptors[0].length = 64;
        descriptors[0].status = super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP;
        assert_eq!(queue.poll_batch(&mut regs, &descriptors, &mut completions), Ok(1));
        let mut backup = [RxDescriptor::default(); 1];
        regs.fail_register = Some(REG_RDT);
        assert_eq!(queue.recycle_batch(&mut regs, &mut descriptors, &completions, &mut backup),
            Err(E1000QueueError::Register(())));
        assert_eq!(descriptors[0].status, super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP);
        regs.fail_register = None;
        assert_eq!(queue.recycle_batch(&mut regs, &mut descriptors, &completions, &mut backup), Ok(1));
        assert_eq!(descriptors[0].status, 0);
    }

    #[test]
    fn rx_batch_rejects_uncompleted_descriptor_without_partial_rearm() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [RxDescriptor::for_buffer(0xa000); 8];
        let mut queue = E1000RxQueue::new(8).unwrap();
        let mut completions = [(0u16, super::super::e1000_descriptor::RxCompletion {
            length: 64, end_of_packet: true, errors: 0,
        }); 2];
        descriptors[0].status = super::super::e1000::RXD_STATUS_DD;
        descriptors[1].status = 0;
        assert_eq!(queue.poll_batch(&mut regs, &descriptors, &mut completions), Ok(1));
        // Simulate corrupted ownership metadata after polling. The queue must
        // reject the recycle without advancing RDT or changing software state.
        descriptors[0].status = 0;
        let mut backup = [RxDescriptor::default(); 1];
        assert_eq!(queue.recycle_batch(&mut regs, &mut descriptors, &completions[..1], &mut backup),
            Err(E1000QueueError::InvalidRing));
        assert_eq!(descriptors[0].status, 0);
        assert_eq!(regs.writes.iter().filter(|(register, _)| *register == REG_RDT).count(), 0);
    }

    #[test]
    fn refuses_to_initialize_without_bus_mastering() {
        let mut regs = MockRegisters::default();
        let queues = E1000QueueConfig::defaults(0x1000, 0x2000).unwrap();
        let mut rx = [RxDescriptor::default(); 32];
        let mut rx_storage = [[0u8; 2048]; 32];
        let rx_buffers: Vec<DmaBuffer<'_>> = rx_storage.iter_mut().enumerate()
            .map(|(index, bytes)| mock_dma_buffer(0x4000 + index as u64 * 2048, bytes))
            .collect();
        let mut tx = [TxDescriptor::default(); 32];
        // SAFETY: MockRegisters does not perform DMA; these buffers model pinned mappings.
        assert_eq!(unsafe { initialize(&mut regs, queues, &mut rx, &rx_buffers, &mut tx, false) }, Err(E1000InitError::BusMasterDisabled));
        assert!(regs.writes.is_empty());
    }
