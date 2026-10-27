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
    for i in 0..pn {
        let p = ph + i * ps;
        if word(bytes, p)? != 1 {
            continue;
        } // PT_LOAD
        let address = word(bytes, p + 12)?;
        let size = word(bytes, p + 16)?;
        let offset = word(bytes, p + 4)? as usize;
        if offset > bytes.len() || size as usize > bytes.len() - offset {
            return Err("Truncated ELF load segment".into());
        }
        if size == 0 {
            continue;
        }
        if address < 0x800000 {
            flash = flash.max(address.checked_add(size).ok_or("ELF address overflow")?);
        } else if !(0x810000..0x860000).contains(&address) {
            return Err(format!("Unsupported ELF load address 0x{address:x}"));
        }
    }
    let base = 0x800000 + target.sram_start;
    let mut sram = 0;
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
        if (base..0x810000).contains(&address) {
            sram = sram.max(end - base);
        } else if address >= 0x800000 && !(0x810000..0x860000).contains(&address) {
            return Err(format!("Unsupported ELF allocated address 0x{address:x}"));
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
            (96, 1),
            (100, 400),
            (108, 100),
            (112, data_size),
            (176, 2),
            (180, 0x800100),
            (188, data_size),
            (216, 2),
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
        assert!(from_elf(&oversized, &ATMEGA328P)
            .unwrap_err()
            .contains("capacity exceeded"));
    }
}
