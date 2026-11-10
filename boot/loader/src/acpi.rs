//! Step 6: the ACPI 2.0 table pointer, which must be fetched before
//! `ExitBootServices`. The loader only passes it on.

use uefi::table::cfg::ConfigTableEntry;

pub fn rsdp() -> Option<u64> {
    uefi::system::with_config_table(|entries| {
        entries
            .iter()
            .find(|entry| entry.guid == ConfigTableEntry::ACPI2_GUID)
            .map(|entry| entry.address as u64)
    })
    .filter(|&address| address != 0)
}
