//! Defines types and probe methods of all supported devices.

#![allow(unused_imports, dead_code)]

use crate::AxDeviceEnum;
use axdriver_base::DeviceType;

#[cfg(feature = "virtio")]
use crate::virtio::{self, VirtIoDevMeta};

#[cfg(feature = "bus-pci")]
use axdriver_pci::{DeviceFunction, DeviceFunctionInfo, PciRoot};

pub use super::dummy::*;

pub trait DriverProbe {
    fn probe_global() -> Option<AxDeviceEnum> {
        None
    }

    #[cfg(bus = "mmio")]
    fn probe_mmio(_mmio_base: usize, _mmio_size: usize) -> Option<AxDeviceEnum> {
        None
    }

    #[cfg(bus = "pci")]
    fn probe_pci(
        _root: &mut PciRoot,
        _bdf: DeviceFunction,
        _dev_info: &DeviceFunctionInfo,
    ) -> Option<AxDeviceEnum> {
        None
    }
}

#[cfg(net_dev = "virtio-net")]
register_net_driver!(
    <virtio::VirtIoNet as VirtIoDevMeta>::Driver,
    <virtio::VirtIoNet as VirtIoDevMeta>::Device
);

#[cfg(block_dev = "virtio-blk")]
register_block_driver!(
    <virtio::VirtIoBlk as VirtIoDevMeta>::Driver,
    <virtio::VirtIoBlk as VirtIoDevMeta>::Device
);

#[cfg(display_dev = "virtio-gpu")]
register_display_driver!(
    <virtio::VirtIoGpu as VirtIoDevMeta>::Driver,
    <virtio::VirtIoGpu as VirtIoDevMeta>::Device
);

cfg_if::cfg_if! {
    if #[cfg(block_dev = "ramdisk")] {
        pub struct RamDiskDriver;
        register_block_driver!(RamDiskDriver, axdriver_block::ramdisk::RamDisk);

        impl DriverProbe for RamDiskDriver {
            fn probe_global() -> Option<AxDeviceEnum> {
                // TODO: format RAM disk
                Some(AxDeviceEnum::from_block(
                    axdriver_block::ramdisk::RamDisk::new(0x100_0000), // 16 MiB
                ))
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(block_dev = "bcm2835-sdhci")]{
        pub struct BcmSdhciDriver;
        register_block_driver!(MmckDriver, axdriver_block::bcm2835sdhci::SDHCIDriver);

        impl DriverProbe for BcmSdhciDriver {
            fn probe_global() -> Option<AxDeviceEnum> {
                debug!("mmc probe");
                axdriver_block::bcm2835sdhci::SDHCIDriver::try_new().ok().map(AxDeviceEnum::from_block)
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(net_dev = "ixgbe")] {
        use crate::ixgbe::IxgbeHalImpl;
        use axhal::mem::phys_to_virt;
        pub struct IxgbeDriver;
        register_net_driver!(IxgbeDriver, axdriver_net::ixgbe::IxgbeNic<IxgbeHalImpl, 1024, 1>);
        impl DriverProbe for IxgbeDriver {
            #[cfg(bus = "pci")]
            fn probe_pci(
                    root: &mut axdriver_pci::PciRoot,
                    bdf: axdriver_pci::DeviceFunction,
                    dev_info: &axdriver_pci::DeviceFunctionInfo,
                ) -> Option<crate::AxDeviceEnum> {
                    use axdriver_net::ixgbe::{INTEL_82599, INTEL_VEND, IxgbeNic};
                    if dev_info.vendor_id == INTEL_VEND && dev_info.device_id == INTEL_82599 {
                        // Intel 10Gb Network
                        info!("ixgbe PCI device found at {:?}", bdf);

                        // Initialize the device
                        // These can be changed according to the requirments specified in the ixgbe init function.
                        const QN: u16 = 1;
                        const QS: usize = 1024;
                        let bar_info = root.bar_info(bdf, 0).unwrap();
                        match bar_info {
                            axdriver_pci::BarInfo::Memory {
                                address,
                                size,
                                ..
                            } => {
                                let ixgbe_nic = IxgbeNic::<IxgbeHalImpl, QS, QN>::init(
                                    phys_to_virt((address as usize).into()).into(),
                                    size as usize
                                )
                                .expect("failed to initialize ixgbe device");
                                return Some(AxDeviceEnum::from_net(ixgbe_nic));
                            }
                            axdriver_pci::BarInfo::IO { .. } => {
                                error!("ixgbe: BAR0 is of I/O type");
                                return None;
                            }
                        }
                    }
                    None
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(net_dev = "igb")] {
        use axdma::{alloc_coherent, dealloc_coherent, BusAddr, DMAInfo};
        use igb_driver::IgbHal;
        use axhal::mem::{phys_to_virt, virt_to_phys};
        use core::{alloc::Layout, ptr::NonNull};
        pub struct IgbHalImpl;
        unsafe impl IgbHal for IgbHalImpl {
            fn dma_alloc(size: usize) -> (usize, NonNull<u8>) {
                let layout = Layout::from_size_align(size, 8).unwrap();
                match unsafe { alloc_coherent(layout) } {
                    Ok(dma_info) => (dma_info.bus_addr.as_u64() as usize, dma_info.cpu_addr),
                    Err(_) => (0, NonNull::dangling()),
                }
            }

            unsafe fn dma_dealloc(paddr: usize, vaddr: NonNull<u8>, size: usize) -> i32 {
                let layout = Layout::from_size_align(size, 8).unwrap();
                let dma_info = DMAInfo {
                    cpu_addr: vaddr,
                    bus_addr: BusAddr::from(paddr as u64),
                };
                unsafe { dealloc_coherent(dma_info, layout) };
                0
            }

            unsafe fn mmio_phys_to_virt(paddr: usize, _size: usize) -> NonNull<u8> {
                NonNull::new(phys_to_virt(paddr.into()).as_mut_ptr()).unwrap()
            }

            unsafe fn mmio_virt_to_phys(vaddr: NonNull<u8>, _size: usize) -> usize {
                virt_to_phys((vaddr.as_ptr() as usize).into()).into()
            }

            fn wait_until(duration: core::time::Duration) -> Result<(), &'static str> {
                axhal::time::busy_wait_until(duration);
                Ok(())
            }
        }

        pub struct IgbDriver;
        const QN: u16 = 1;
        const QS: usize = 1024;
        register_net_driver!(IgbDriver, igb_driver::IgbNic<IgbHalImpl, QS, QN>);
        impl DriverProbe for IgbDriver {
            #[cfg(bus = "pci")]
            fn probe_pci(
                root: &mut axdriver_pci::PciRoot,
                bdf: axdriver_pci::DeviceFunction,
                dev_info: &axdriver_pci::DeviceFunctionInfo,
            ) -> Option<crate::AxDeviceEnum> {
                use igb_driver::{INTEL_82576, INTEL_VEND};
                use igb_driver::IgbNic;
                if dev_info.vendor_id == INTEL_VEND && dev_info.device_id == INTEL_82576 {
                    info!("igb PCI device found at {:?}", bdf);

                    // Initialize the device
                    // These can be changed according to the requirements specified in the igb init function.
                    let bar_info = root.bar_info(bdf, 0).unwrap();
                    match bar_info {
                        axdriver_pci::BarInfo::Memory { address, size, .. } => {
                            let igb_nic = IgbNic::<IgbHalImpl, QS, QN>::init(
                                phys_to_virt((address as usize).into()).into(),
                                size as usize
                            )
                            .expect("failed to initialize igb device");
                            return Some(AxDeviceEnum::from_net(igb_nic));
                        }
                        axdriver_pci::BarInfo::IO { .. } => {
                            error!("igb: BAR0 is of I/O type");
                            return None;
                        }
                    }
                }
                None
            }
        }
    }
}
