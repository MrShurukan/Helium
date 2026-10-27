//! Final linked ELF accounting, not a source-level estimate. AVR ELF uses a
//! 0x800000 bias for data-space addresses. Stack bounds remain explicitly unknown.
use super::microcontroller::Microcontroller;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReport {
    /// Highest flash load address plus one, including alignment gaps.
    pub flash_extent: u32,
    /// SRAM reserved through the end of the highest allocated section.
    pub static_sram: u32,
    pub flash_capacity: u32,
    pub sram_capacity: u32,
}

impl MemoryReport {
    pub fn render(&self) -> String {
        format!(
            "Memory (linked ELF; whole MCU, bootloader space not reserved):\n  Flash occupied: {} / {} bytes\n  Static SRAM reserved: {} / {} bytes\n  SRAM remaining for stack: {} bytes (not a stack safety guarantee)\n  Maximum stack: unknown (machine call frames, runtime helpers and ISR overlap not bounded)\n  Timing: ISR duration and interrupt-masking latency unknown; timer periods are nominal\n",
            self.flash_extent, self.flash_capacity, self.static_sram,
            self.sram_capacity, self.sram_capacity.saturating_sub(self.static_sram)
        )
    }
}

fn word(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or("Truncated ELF header/table")?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn half(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or("Truncated ELF header/table")?;
    Ok(u16::from_le_bytes(value.try_into().unwrap()) as usize)
}

fn table(
    bytes: &[u8],
    offset: usize,
    stride: usize,
    count: usize,
    minimum: usize,
) -> Result<(), String> {
    if count == 0
        || stride < minimum
        || offset > bytes.len()
        || count > (bytes.len() - offset) / stride
    {
        return Err("Missing, unsupported or truncated ELF table".into());
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Flash,
    Sram,
    Special(u32),
}

fn region(address: u32, size: u32, target: &Microcontroller) -> Result<Region, String> {
    let end = address.checked_add(size).ok_or("ELF address overflow")?;
    let (region, limit) = if address < 0x800000 {
        (Region::Flash, 0x800000)
    } else if (0x800000 + target.sram_start..0x810000).contains(&address) {
        (Region::Sram, 0x810000)
    } else if (0x810000..0x860000).contains(&address) {
        (
            Region::Special(address >> 16),
            (address & !0xffff) + 0x10000,
        )
    } else {
        return Err(format!("Unsupported ELF address 0x{address:x}"));
    };
    if end > limit {
        return Err("ELF range crosses address-space boundary".into());
    }
    Ok(region)
}

struct Load {
    offset: u32,
    address: u32,
    file_size: u32,
    memory_size: u32,
}

fn file_range(bytes: &[u8], offset: u32, size: u32) -> Result<(), String> {
    let offset = offset as usize;
    if offset > bytes.len() || size as usize > bytes.len() - offset {
        return Err("Truncated ELF load segment or section".into());
    }
    Ok(())
}

pub fn from_elf(bytes: &[u8], target: &Microcontroller) -> Result<MemoryReport, String> {
    if bytes.get(..7) != Some(b"\x7fELF\x01\x01\x01")
        || half(bytes, 16)? != 2
        || half(bytes, 18)? != 83
    {
        return Err("Memory report requires a linked little-endian ELF32 AVR executable".into());
    }
    let ph = word(bytes, 28)? as usize;
    let sh = word(bytes, 32)? as usize;
    let ps = half(bytes, 42)?;
    let pn = half(bytes, 44)?;
    let ss = half(bytes, 46)?;
    let sn = half(bytes, 48)?;
    table(bytes, ph, ps, pn, 32)?;
    table(bytes, sh, ss, sn, 40)?;
    let mut flash = 0;
    let mut sram = 0;
    let base = 0x800000 + target.sram_start;
    let mut loads = Vec::new();
    for i in 0..pn {
        let p = ph + i * ps;
        if word(bytes, p)? != 1 {
            continue;
        } // PT_LOAD
        let address = word(bytes, p + 12)?;
        let size = word(bytes, p + 16)?;
        let offset = word(bytes, p + 4)?;
        let virtual_address = word(bytes, p + 8)?;
        let memory_size = word(bytes, p + 20)?;
        if size > memory_size {
            return Err("ELF segment file size exceeds memory size".into());
        }
        file_range(bytes, offset, size)?;
        if memory_size == 0 {
            continue;
        }
        let runtime = region(virtual_address, memory_size, target)?;
        if runtime == Region::Sram {
            sram = sram.max(virtual_address + memory_size - base);
        }
        if size != 0 {
            let storage = region(address, size, target)?;
            match (runtime, storage) {
                (Region::Flash, Region::Flash) if address == virtual_address => {}
                (Region::Sram, Region::Flash) => {}
                (Region::Special(a), Region::Special(b))
                    if a == b && address == virtual_address => {}
                _ => return Err("Unsupported ELF load/runtime address mapping".into()),
            }
            if storage == Region::Flash {
                flash = flash.max(address + size);
            }
        }
        loads.push(Load {
            offset,
            address: virtual_address,
            file_size: size,
            memory_size,
        });
    }
    if flash == 0 {
        return Err("Missing nonempty Flash PT_LOAD segment".into());
    }
    for i in 0..sn {
        let s = sh + i * ss;
        if word(bytes, s + 8)? & 2 == 0 {
            continue;
        } // SHF_ALLOC
        let address = word(bytes, s + 12)?;
        let size = word(bytes, s + 20)?;
        if size == 0 {
            continue;
        }
        let end = address
            .checked_add(size)
            .ok_or("ELF section address overflow")?;
        let area = region(address, size, target)?;
        let section_type = word(bytes, s + 4)?;
        let offset = word(bytes, s + 16)?;
        let nobits = section_type == 8;
        if section_type == 0 || (nobits && area != Region::Sram) {
            return Err("Unsupported allocated ELF section type".into());
        }
        if !nobits {
            file_range(bytes, offset, size)?;
        }
        let covered = loads.iter().any(|load| {
            let Some(delta) = address.checked_sub(load.address) else {
                return false;
            };
            u64::from(delta) + u64::from(size) <= u64::from(load.memory_size)
                && (nobits
                    || (u64::from(delta) + u64::from(size) <= u64::from(load.file_size)
                        && u64::from(offset) == u64::from(load.offset) + u64::from(delta)))
        });
        if !covered {
            return Err(
                "Allocated ELF section is not covered by a consistent PT_LOAD segment".into(),
            );
        }
        if area == Region::Sram {
            sram = sram.max(end - base);
        }
    }
    if flash > target.flash_bytes || sram > target.sram_bytes {
        return Err(format!("MCU memory capacity exceeded: Flash {flash}/{} bytes, static SRAM {sram}/{} bytes; stack not included", target.flash_bytes, target.sram_bytes));
    }
    Ok(MemoryReport {
        flash_extent: flash,
        static_sram: sram,
        flash_capacity: target.flash_bytes,
        sram_capacity: target.sram_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::microcontroller::ATMEGA328P;
    fn fixture(data_size: u32, bss_size: u32) -> Vec<u8> {
        let mut b = vec![0; 512];
        b[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
        for (p, v) in [(16, 2u16), (18, 83), (42, 32), (44, 2), (46, 40), (48, 3)] {
            b[p..p + 2].copy_from_slice(&v.to_le_bytes());
        }
        for (p, v) in [
            (28, 64u32),
            (32, 128),
            (64, 1),
            (68, 300),
            (76, 0),
            (80, 100),
            (84, 100),
            (96, 1),
            (100, 400),
            (104, 0x800100),
            (108, 100),
            (112, data_size),
            (116, 16 + bss_size),
            (172, 1),
            (176, 2),
            (180, 0x800100),
            (184, 400),
            (188, data_size),
            (216, 2),
            (212, 8),
            (220, 0x800110),
            (228, bss_size),
        ] {
            b[p..p + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
    #[test]
    fn counts_data_twice_and_bss_with_alignment_gap() {
        let r = from_elf(&fixture(4, 8), &ATMEGA328P).unwrap();
        assert_eq!(r.flash_extent, 104);
        assert_eq!(r.static_sram, 24);
        assert!(r.render().contains("Maximum stack: unknown"));
        assert!(r.render().contains("2024 bytes"));
    }
    #[test]
    fn capacity_and_malformed_inputs_fail_closed() {
        assert!(from_elf(&fixture(4, 2048), &ATMEGA328P)
            .unwrap_err()
            .contains("capacity exceeded"));
        let b = fixture(4, 8);
        for length in 0..248 {
            assert!(from_elf(&b[..length], &ATMEGA328P).is_err());
        }
        let mut oversized = b;
        oversized[76..80].copy_from_slice(&32768u32.to_le_bytes());
        oversized[72..76].copy_from_slice(&32768u32.to_le_bytes());
        assert!(from_elf(&oversized, &ATMEGA328P)
            .unwrap_err()
            .contains("capacity exceeded"));
    }

    #[test]
    fn rejects_inconsistent_sections_segments_and_address_spaces() {
        for (offset, value) in [
            (64, 0),         // No Flash LOAD.
            (176, 2),        // A valid flag, followed below by a deliberately bad section address.
            (184, 401),      // File/virtual offsets disagree.
            (188, 200),      // Section contents truncated.
            (180, 0x800200), // SRAM section outside LOAD.
            (116, 2),        // File size exceeds memory size.
            (104, 0x80fff0), // Segment crosses SRAM address space.
            (108, 0x80fffe), // File image in SRAM is unsupported.
        ] {
            let mut b = fixture(4, 32);
            b[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
            if offset == 64 {
                b[96..100].copy_from_slice(&0u32.to_le_bytes());
            }
            if offset == 176 {
                b[180..184].copy_from_slice(&0u32.to_le_bytes());
                b[188..192].copy_from_slice(&40000u32.to_le_bytes());
            }
            assert!(from_elf(&b, &ATMEGA328P).is_err(), "offset {offset}");
        }
    }
}
