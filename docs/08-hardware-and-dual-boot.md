# 08 · Hardware testing and dual boot (Milestone 5 plan)

Status: **plan only. Nothing in this document is to be carried out until Milestones 1–4 are accepted and the machine's owner has reviewed the machine-specific steps.** No command here has been run on a real machine, with one exception: the help text of `manage-bde -protectors -get -?` was read while this plan was written. That changes nothing and prints no key. Windows behaviour is cited to Microsoft documentation and firmware behaviour to the UEFI Specification 2.11; links are in [09](09-references.md#windows-secure-boot-and-bitlocker). Where Microsoft's documentation is silent, the text says so.

## 1. Safety rules

1. **Read-only first.** Stage 5a only reads. It changes no setting, mounts nothing, and writes no file outside the project folder. The report never contains recovery passwords or keys.
2. **Never write to the Windows EFI System Partition**, the Microsoft Reserved partition, the Windows partition, or any recovery partition. Microsoft describes the system partition as managed by the operating system. The one exception is the last-resort repair of Windows' own boot files with Microsoft's tools in section 8, and it needs the machine owner's explicit go-ahead.
3. **Never change the internal disk's partition table** in this milestone.
4. **Removable media before internal drives; a separate drive before a shared one.** An external USB drive, which can be unplugged, is preferred over an internal second drive (D-H-4).
5. **BitLocker before firmware.** Before any firmware setting, Secure Boot key, or boot-entry change: confirm the recovery key is retrievable and matches, then `Suspend-BitLocker -MountPoint C: -RebootCount 0` (no automatic resume). Resume by hand only when the firmware state is final and Windows has started once in that state. While suspended the volume key is stored unprotected on the disk: keep the window short and keep the machine in your hands. If the inventory shows the volume fully decrypted, there is nothing to suspend and the suspend and resume steps are skipped. If it shows "encrypted but waiting for activation", no firmware change is made until you have either finished activation and saved the recovery key or decrypted the volume, because protection could otherwise arm itself in the middle of a change.
6. **Stillkey cannot write to a disk or to firmware settings in this milestone, and two kinds of test must agree on it before any hardware boot.** Stillkey boots entirely from files that firmware reads into memory. The kernel has no storage driver and never calls UEFI runtime services. The loader, which runs while firmware's write-capable services exist, opens files read-only and contains no call to file write, delete, flush or set-info, to Block I/O write, or to `SetVariable`; the only runtime service it calls is `ResetSystem(EfiResetShutdown)` (rule 9). This is the strongest protection for Windows and it is a property of the build, not of care taken during use. The evidence for it:
   - **The scan, one check.** `cargo xtask test m5-readonly` fails the build if the loader's sources or symbols reference any of the write-capable calls. The source scan and the symbol scan look for the same thing in two forms of the same program, so together they count as **one** check, not two. A scan shows that a name is absent; it does not show what the running loader does (a call through a raw function pointer has no name), so it is never sufficient alone.
   - **Boots on disposable VM disks.** `cargo xtask test m5-vmdisk` runs the hardware image in QEMU with two disk images attached writable: the boot image, and a second disposable image that stands in for a Windows disk (GPT, an EFI System Partition with placeholder files, a data partition). It covers **successful boots** (a normal boot, and one with a forced stale map key, `test=stalekey`) and the **loader failure paths** of [02 §11](02-boot-and-handoff.md#11-failure-behaviour): kernel file missing (`KERNEL_MISSING`), each corrupted kernel of `m1-reject` and a kernel above the size limit (`KERNEL_REJECTED`), and from stage 5c a signature that does not verify (`VERIFY_FAILED`). A failure that no image can provoke (`NO_MEMORY`, `EBS_FAILED`, `CPU_UNSUPPORTED`) gets a test-only boot option if one can be added without changing the path under test; otherwise the test's result line names it as not covered. After every run the SHA-256 of each disk image must equal its value before the run. Firmware writes its own variables during any boot, so the per-run copy of the variable store is not compared byte for byte; for `SetVariable` the scan and the review of the loader's short list of firmware calls remain the evidence.
   
   Both must pass on the loader binary that goes onto removable media: the stage 5b procedure compares the SHA-256 of the file it writes with the one these tests ran on. The disk images are files under `~/.cache/stillkey/`; no test ever opens a physical device. Milestone 1 supplies the scenarios: `m1-boot` and `m1-stalekey` (successful boots) run on disposable images today, and `m1-reject` (a failure path) is specified in [07 §9](07-milestone-1-spec.md#9-acceptance-tests). `m5-vmdisk` adds the second disk and the comparison of every image before and after.
7. **Every step has a written undo** before it is performed.
8. **One named tool writes to physical devices:** `tools/write-usb.ps1`, run by you in an elevated Windows PowerShell. Nothing inside WSL or QEMU ever opens a physical device. Every write requires positive identification of the device by bus type, size, serial number and unique ID, and a typed confirmation (section 3).
9. **Windows never starts in the same power-on in which the Stillkey loader ran.** On hardware the loader shuts the machine down after a failure instead of returning to firmware (D-H-4), and after every Stillkey session the machine is powered fully off before Windows is started. Section 3 gives the reason.

## 2. What we need to know about the machine first

Machine-specific steps will not be written until this table is filled in. The assumption that the target is **x86-64 with UEFI firmware** (A-01) is verified here.

| Item | Why it matters | Read-only source | Needs administrator |
|---|---|---|---|
| Maker, model, SKU | Firmware menus and keys differ by model | `Get-CimInstance Win32_ComputerSystem` | No |
| Windows edition and build | Decides whether the machine has BitLocker or Device Encryption, and which cmdlets exist | `Get-ComputerInfo -Property OsName,OsVersion,OsBuildNumber,WindowsEditionId` | No |
| Firmware vendor, version, date | Known quirks; whether an update is pending | `Get-CimInstance Win32_BIOS` | No |
| Processor model | 64-bit, no-execute, 1 GiB pages, core count | `Get-CimInstance Win32_Processor` | No |
| Firmware boot mode | Must be UEFI, not legacy | `Get-ComputerInfo -Property BiosFirmwareType` (documented). `msinfo32` "BIOS Mode" as an informal cross-check (not documented by Microsoft) | No |
| Secure Boot state | Decides the trust route in section 5 | `Confirm-SecureBootUEFI`; `msinfo32` "Secure Boot State" | Cmdlet: yes |
| Secure Boot keys present | Which Microsoft certificates firmware trusts; whether firmware is in user mode (`SetupMode` 0 = a platform key is enrolled, UEFI §32.3) | `Get-SecureBootUEFI -Name db` (search for "Windows UEFI CA 2023", "Microsoft UEFI CA 2023", "Microsoft Corporation UEFI CA 2011"); `-Name SetupMode`. Whether custom enrolment exists is read from firmware setup, not from this variable | Yes |
| Firmware default keys and certificate servicing state | Whether "restore factory keys" would take away the 2023 certificates that Windows now starts with (section 8) | `Get-SecureBootUEFI -Name dbDefault` and `-Name KEKDefault`; `HKLM:\SYSTEM\CurrentControlSet\Control\SecureBoot\Servicing` value `UEFICA2023Status` | Yes |
| Disks and bus types | How many drives; NVMe, SATA or RAID (RAID suggests Intel RST/VMD in front of the drive) | `Get-PhysicalDisk \| Select DeviceID,FriendlyName,SerialNumber,BusType,MediaType,Size` | Confirm on the machine |
| Disk identity for rule 8 | Tells the Windows disk apart from any disk we may write | `Get-Disk \| Select Number,UniqueId,SerialNumber,BusType,IsBoot,IsSystem,Size` | No |
| Partition style and layout | GPT required; identify ESP, MSR, Windows, recovery by type GUID | `Get-Disk`; `Get-Partition \| Select DiskNumber,PartitionNumber,Type,GptType,Size` | Confirm on the machine |
| Recovery environment | Where Windows RE lives, so we never disturb it | `reagentc /info` | Yes in practice |
| BitLocker or Device Encryption | Whether changes can trigger recovery | `Get-BitLockerVolume \| Select MountPoint,VolumeStatus,ProtectionStatus,LockStatus,EncryptionPercentage`; Settings › Privacy & security › Device encryption | Yes in practice |
| Encryption state in detail | Device Encryption can be encrypted but not yet armed: until a Microsoft account signs in the key is unprotected, no recovery key exists, and protection can arm itself in the middle of the project | `manage-bde -status C:`: Protection On / Off (suspended) / encrypted but waiting for activation / decrypted | Yes |
| Which TPM measurements protect the key | With Secure Boot binding (PCR 7 and 11) BitLocker reacts to Secure Boot changes; without it (PCR 0, 2, 4, 11) it also reacts to firmware and boot-manager changes | `manage-bde -protectors -get C: -Type TPM` (shows only the TPM protector and its PCR Validation Profile). If `manage-bde -status C:` lists the protector as TPM with a PIN, the type to ask for is `-Type TPMAndPIN`; the script takes the type from the status output and never asks without `-Type`. **Never run `manage-bde -protectors -get C:` without a `-Type` filter that names a TPM protector type (`TPM` or `TPMAndPIN`), and never print the `RecoveryPassword` property of `Get-BitLockerVolume` key protectors: both reveal the recovery password** | Yes |
| PCR 7 binding | Whether Windows reports that binding BitLocker to the Secure Boot state is possible on this firmware, and if not, why | `msinfo32` as administrator: "PCR7 Configuration" and "Device Encryption Support" | Yes |
| Recovery key location | Must be retrievable before any change | Microsoft account (`aka.ms/myrecoverykey`), work account, printout or file; compare the first 8 digits of its key ID with the output of `(Get-BitLockerVolume -MountPoint C:).KeyProtector \| Where-Object KeyProtectorType -eq 'RecoveryPassword' \| Select-Object -ExpandProperty KeyProtectorId`, which prints the ID only | Reading the ID: yes |
| TPM | Present and ready | `Get-Tpm` | Yes in practice |
| Firmware boot entries | Baseline for before-and-after comparison | `bcdedit /enum firmware` | Yes |
| Fast Startup and hibernation | See section 9 | `powercfg /a` (which sleep states are available) and `Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' -Name HiberbootEnabled` (1 = Fast Startup is on) | No |
| Pending updates | A Windows or firmware update during the test window restarts the machine and can change the Secure Boot databases | Settings › Windows Update shows nothing pending | No |
| Built-in keyboard type | The demo needs a PS/2-style keyboard (most laptop built-in keyboards); a USB-only desktop cannot drive the desktop yet | Device Manager › Keyboards ("Standard PS/2 Keyboard" or "HID Keyboard Device") | No |
| Display | Panel resolution; which output firmware uses | Settings › Display; observed at firmware boot | No |
| Firmware keys | One-time boot menu key, setup key | Maker's documentation | – |
| Does firmware allow: Secure Boot off; custom key or hash enrolment; a third-party CA switch | Microsoft's published UEFI requirements make switching Secure Boot off and custom-mode enrolment optional for the maker, so none can be assumed | Read from firmware setup; photograph the pages | – |
| Firmware setup facts | Each is relied on by a later step: enabling USB boot is a firmware change under rule 5; section 7.1 and the rollback use "add boot option" and "boot from file"; route A needs a plain switch | Photograph the pages: is an administrator password set; is USB boot enabled; is there "add boot option" and "boot from file"; is Secure Boot a plain on/off switch or tied to clearing keys | – |
| Serial or other debug output | Almost certainly none; plan for on-screen logs | Inspection | – |

Commands that look harmless but are **not** read-only and are excluded from stage 5a: `mountvol X: /S` (mounts the EFI partition), anything that writes `bcdedit`, `reagentc /enable|/disable`, `manage-bde -protectors -disable`, `Suspend-BitLocker`, `powercfg /h on|off`, pausing Windows updates. Also excluded, although read-only, because they print the recovery password: `manage-bde -protectors -get C:` without a `-Type` filter that names a TPM protector type, and any listing of BitLocker key protectors that includes the `RecoveryPassword` property.

Deliverable 5a is a script, `tools/host-report.ps1`, that runs only the commands above and marks which ones needed elevation. It never invokes a command or selects a property that can output a recovery password; only protector types, protector IDs and the PCR validation profile reach the report. It aborts without writing if its output matches the 48-digit recovery password pattern (`\d{6}(-\d{6}){7}`), redacts serial numbers on request, and writes `build/host-report.txt`, which git ignores because the whole `build/` folder is in `.gitignore`.

## 3. Stage 5b: removable-media test through the firmware boot menu

Goal: boot Stillkey on the real machine without writing to any of its disks. If Secure Boot is on, this stage also needs one reviewed, reversible firmware change (route A or B) and the full BitLocker procedure.

1. `cargo xtask image --hardware` builds the USB image: GPT, one FAT32 EFI System Partition, the loader at `\EFI\BOOT\BOOTX64.EFI` (the default removable-media path, UEFI §3.5.1.1), and the kernel, the boot filesystem and the signed manifest under `\EFI\STILLKEY\`. The flag builds the loader with the cargo feature `hardware` (section 4.1), and the image carries no `qemu_exit=on` option. `xtask` produces an image file only.
2. You write the image to a USB stick that contains nothing you need, with `tools/write-usb.ps1` (described below). This is the first physical write; rule 8 applies.
3. Before the machine is started from the stick, confirm that the gate which precedes this stage has been passed (section 10): recovery key located and its ID matched; the section 8 preparations complete; the rollback rehearsed on this machine. If any of the three is missing, stop here.
4. If a firmware change is needed, which is the case when Secure Boot is on or the inventory shows USB boot switched off: `Suspend-BitLocker -MountPoint C: -RebootCount 0`, make the change in firmware setup (the route A or B change of section 5; switching USB boot on), and start Windows once to confirm it starts.
5. Windows is **restarted** (not shut down; see section 9) and the firmware's one-time boot menu is used to pick the USB stick. We do not change the stored boot order; whether firmware changed it is checked afterwards with `bcdedit /enum firmware`. Open the menu with the firmware hot key, not with Windows' "Use a device" option. We assume that option works by setting the one-shot `BootNext` variable (UEFI §3.1.2), which would be a write to a firmware variable; Microsoft's documentation does not say how it works.
6. Expected results, in order of likelihood on first try: a readable Stillkey diagnostic screen naming what is missing; the kernel's log on the screen without a usable desktop; the desktop. A loader failure is drawn on the screen; the loader then waits for a key press and shuts the machine down.
7. Power the machine fully off, remove the stick, then power on. Windows must never start in the same power-on in which the Stillkey loader ran (rule 9). If a BitLocker recovery screen appears straight after a Stillkey attempt, power off and start again before entering any key.
8. If step 4 was done: with route A, switch Secure Boot back on and start Windows again. Then, whatever the change was, `Resume-BitLocker -MountPoint C:` with the checks in the section 8 rows. In every case re-run the host report. Windows starts as before.

**The writer.** `tools/write-usb.ps1` lists `Get-Disk | Select Number,FriendlyName,SerialNumber,UniqueId,BusType,Size,PartitionStyle,IsBoot,IsSystem` and shows the chosen disk's partitions and volume labels. It refuses unless all of these hold: BusType is USB; IsBoot and IsSystem are False; the size is at most 256 GB; the disk is not the one holding `C:` or the project folder; the disk holds no Microsoft Reserved partition, no recovery partition and no BitLocker volume; and it holds no EFI System Partition, unless that is the disk's only partition (the layout this tool itself writes). It then requires you to type the serial number (or, if none is reported, the exact size in bytes and the model), re-reads `UniqueId` immediately before opening the device and aborts on a mismatch, locks and dismounts the stick's volumes, writes, reads back and compares SHA-256 with the image. A raw write from WSL is not possible in any case: `wsl --mount` needs administrator rights, attaches whole disks by `\\.\PHYSICALDRIVEn` number and does not support USB sticks.

**Why the power-off matters.** Firmware that gets an error back from a boot option goes on to the next entry in its boot order (UEFI §3.1.1), which is Windows Boot Manager. By then the TPM has measured the Stillkey loader and, under route B, the allow-list entry that authorised it. BitLocker then sees a boot chain it did not seal its key against and asks for the recovery key; Microsoft describes the same effect for a failed network boot that falls back to Windows (KB5085046). So the loader never returns to firmware on hardware: it draws the failure line, waits for a key press and calls `ResetSystem(EfiResetShutdown)`. Returning an error status is kept for VM tests only.

**Secure Boot applies to removable media and one-time boots exactly as to anything else.** If Secure Boot is on, firmware does not start a loader it does not trust (UEFI §32.4.2, §32.5.3.3; Microsoft's UEFI requirements forbid an override by a physically present user). So stage 5b needs one of the routes in section 5 to be chosen first.

What stage 5b proves and does not: it proves the loader, kernel and desktop work on this hardware. It does not test installation.

## 4. Hardware differences we expect to hit

| Area | VM | Real machine | Plan |
|---|---|---|---|
| Debug output | Serial log | Usually no serial port | The kernel draws its log on the framebuffer and the panic banner is on screen; stage 5-prep adds the loader's failure lines |
| Loader failure | Returns an error to firmware, which the test harness sees | Returning would let firmware start Windows in the same power-on | On hardware the loader shuts the machine down after a key press (D-H-4); proven in stage 5-prep |
| Keyboard | PS/2 controller always present | Laptops usually still present one for the built-in keyboard; desktops with USB keyboards do not | Demo on a laptop keyboard first. USB keyboards need a USB host-controller driver, which is a project of its own |
| Timer and interrupt controller | Legacy devices present: the 8254 timer through the 8259 | Legacy timer may be disabled or absent; interrupt routing described by ACPI | Stage 5-prep: ticks come from the local APIC timer and interrupt routing is read from the ACPI MADT. The 8254 is never used on hardware |
| Graphics mode | 1280×800, 32-bit, blue-green-red order | Panel's native resolution; Microsoft's UEFI requirements ask for 32-bit BGR at the panel's native resolution, but we read the mode instead of assuming | Layout reflows; unsupported formats give a clear message |
| Memory map | Small and tidy | Many regions, firmware quirks | The Milestone 1 kernel's memory summary is the first diagnostic we read on hardware |
| Storage | Not used | Not used | Stillkey has no storage driver in this milestone |
| Other processors | Not started | Not started | Single CPU until after the demo |

### 4.1 Stage 5-prep: proven in the VM before the first hardware boot

Stage 5-prep changes only code and is tested only in the VM. It comes before stage 5b.

- **Timer.** Ticks come from the local APIC timer, calibrated against the ACPI PM timer, falling back to the HPET, or taken from CPUID leaf 15H/16H when the processor reports it there.
- **Interrupt routing.** The I/O APIC, the interrupt source overrides and whether 8259s exist are read from the ACPI MADT; the keyboard interrupt is delivered that way. Only static ACPI tables are read (MADT, FADT, HPET); no AML is interpreted.
- **Loader failure screen.** The loader of a hardware image (next item) draws its failure lines on the framebuffer as well as writing them to the serial port.
- **Loader failure path for hardware.** `cargo xtask image --hardware` builds the loader with the cargo feature `hardware`. That loader contains no write to QEMU's debug-exit port and never returns to firmware: failure line on the framebuffer, wait for a key press, `ResetSystem(EfiResetShutdown)`. In the VM a failure of the hardware image therefore ends QEMU with status 0.

Pass ([06](06-milestones.md)): all Milestone 1–4 tests pass with the 8254 interrupt masked and ticks coming from the local APIC timer; `cargo xtask test m5-readonly` (the scan of sources and symbols, one check) and `cargo xtask test m5-vmdisk` (successful boots and loader failure paths on disposable VM disks, every disk image unchanged; rule 6 of section 1) both pass; and a hardware image with a corrupted kernel shows its `SK:LOADER:` failure line on the screen and, after one key press, powers the VM off (QEMU status 0).

## 5. How firmware comes to trust the Stillkey loader

Background, as of November 2026 (expiry dates: Microsoft KB5062710; boot media: KB5132953; 2023-only signing: the 26 June 2026 update to the Hardware Dev Center signing post; the 25H2 rule: §1.6 of Microsoft's Secure Boot key creation and management guidance): the 2011 Microsoft Secure Boot certificates have all expired (machines continue to boot); new third-party signatures are issued only under **Microsoft UEFI CA 2023**; and for PCs shipped with Windows 11 25H2 or later, Microsoft requires only the Windows certificate in the firmware's allow list, making the third-party certificate optional. Many current PCs therefore do not trust third-party loaders by default.

| Route | What you do | Effect on BitLocker | Realistic for one person? | Notes |
|---|---|---|---|---|
| **A. Secure Boot off for testing** | Switch it off in firmware setup (if the firmware allows; this is an optional feature) | With Secure Boot binding, changing it changes what the TPM measures, so expect a recovery prompt unless protection is suspended first | Yes | Simplest. Lowers Windows' protection while off. Suitable for a short test window, then switch back on. Allowed only where rule 3 below holds |
| **B. Enrol Stillkey's own certificate or the loader's hash in the firmware allow list (`db`)** | Use firmware setup's custom or user mode (optional feature) to add our public certificate, then sign the loader with our key; or add the hash of one specific loader build | Changing `db` changes the Secure Boot measurement; suspend protection first | Yes, where the firmware offers it | **Recommended when available.** Secure Boot stays on; only our signed loader is added. A hash must be re-enrolled for every loader build; a certificate does not |
| **C. A distribution's Microsoft-signed shim plus a Machine Owner Key** | Chain-load through an existing signed shim; enrol our key with its key manager | Shim's own key store is outside firmware `db`, but boot-chain changes can still matter | Possible but fragile | Depends on firmware trusting the right Microsoft third-party certificate (2011 or 2023), which many new and Secured-core PCs do not by default. Getting our own shim signed requires an organisation, an extended-validation certificate and a review process aimed at Linux boot chains |
| **D. Microsoft signs our loader** | Partner Center submission | None | No | Requires an organisation with an extended-validation certificate and audits |

Rules for routes A and B:

1. **Save the current keys first.** In an elevated PowerShell (this reads firmware variables and writes only project files): `foreach ($n in 'PK','KEK','db','dbx') { Get-SecureBootUEFI -Name $n -OutputFilePath "D:\OS\build\secureboot-$n.bin" }`. Also export the keys to a USB stick if firmware setup offers it.
2. **Add one entry, never replace.** Never choose "Clear/Delete all keys", "Delete PK", "Reset to Setup Mode", "Restore/Install default (factory) keys", or any "Replace/Set new" action on `db`. The only key action in this plan is "Append/Add" of one entry to `db`, and deleting that one entry to undo it. UEFI requires firmware to support appending to the signature databases (§32.7.1); clearing the platform key puts firmware into setup mode (§32.3.2).
3. **Route A needs a plain switch.** It is allowed only if firmware has a Secure Boot Enabled/Disabled switch that leaves PK, KEK, `db` and `dbx` untouched. If switching off requires clearing keys, or switching on installs default keys, route A is not used on this machine. After re-enabling, confirm `db` still contains the certificates in the baseline report ("Windows UEFI CA 2023" among them on an updated PC) before resuming BitLocker.
4. **BitLocker stays suspended for the whole window** (safety rule 5 in section 1), with `-RebootCount 0` and a manual resume. A counted suspension is used up by every Windows start, including update restarts, so protection could re-arm before the firmware change is made or while Secure Boot is still off. Assumption: if protection re-arms while Secure Boot is off, the key is sealed to the profile without Secure Boot (PCR 0, 2, 4, 11) and stays there after Secure Boot is switched back on; Microsoft documents the two profiles but not this sequence. The rollback table has a row for it.

In the VM we can rehearse route B with no risk: OVMF's variable store is a file, so we enrol test keys there and prove that an unsigned loader is refused and a signed one runs. This is stage 5c. It uses the Secure Boot firmware build, not the Milestone 1 one: `/usr/share/OVMF/OVMF_CODE_4M.secboot.fd` with `-machine q35,smm=on -global driver=cfi.pflash01,property=secure,value=on`, a per-run copy of `OVMF_VARS_4M.fd` enrolled with a test PK, KEK and `db`, and a signing tool. The packages for enrolment and signing (for example `python3-virt-firmware` and `sbsigntool`) are added to the setup script at stage 5c and recorded by `cargo xtask setup-check`.

To be ready for any route, the loader meets Microsoft's UEFI memory-mitigation rules from Milestone 1: `IMAGE_DLLCHARACTERISTICS_NX_COMPAT` set, section alignment of at least 4 KiB, no section both writable and executable, and no execution from the stack or from freshly allocated memory while firmware's page tables are active. `m1-unit` asserts the three header properties by reading `BOOTX64.EFI` itself (assumption A-16).

**Decision needed at Milestone 5:** A or B, after the inventory shows what the firmware offers.

## 6. How the loader verifies the kernel

Firmware's check ends at the loader. The loader extends it:

1. A **signed manifest**, `\EFI\STILLKEY\MANIFEST.SIG`, lists the SHA-256 of `KERNEL.ELF`, `BOOTFS.IMG` and, if present, `CMDLINE.TXT`, and is signed with an Ed25519 key.
2. The matching **public key is compiled into the loader**. Because firmware verifies the loader as a whole, the key is covered by that check.
3. Before loading anything, the loader verifies the manifest signature, then each file's digest. Any mismatch prints `SK:LOADER:VERIFY_FAILED file=<name>` on the serial port, and the loader of a hardware image draws it on the screen as well; that loader then shuts the machine down after a key press, and in VM tests the loader returns an error to firmware like every other loader refusal. There is no "boot anyway".
4. When verification is enforced, boot options are honoured only if `CMDLINE.TXT` is listed in the manifest, so test switches cannot be injected by editing a text file.
5. Keys: a development key lives in the repository for VM use, in both build profiles, and is refused by every loader built with the cargo feature `hardware` (`cargo xtask image --hardware`), which contains only the release public key. The release private key is generated and kept offline by you; `xtask sign` takes it as a file path and never copies it.

What this gives: a kernel or filesystem image that was altered on the disk does not run. What it does not give: protection if firmware trust is switched off (route A) and someone replaces the loader itself.

## 7. Stage 5d: dual-boot design

### 7.1 Preferred: a separate drive

- A second drive carries its own GPT and its own FAT32 EFI System Partition (type GUID `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`). The UEFI specification puts no limit on the number or location of such partitions (§13.3.3). An external USB drive, which can be unplugged, is preferred over an internal second drive (D-H-4).
- Files: `\EFI\BOOT\BOOTX64.EFI` and `\EFI\STILLKEY\…` on that partition only.
- **We write no boot entry.** Whether firmware lists the drive, and whether it creates or reorders an entry of its own, is firmware-specific: UEFI §3.1.1 allows the boot manager to maintain its variables automatically, and §3.4.3 and §3.5.1.1 only guarantee the default path for removable media and when nothing else boots. Therefore:
  1. The first time the drive is connected, start Windows and compare `bcdedit /enum firmware` with the baseline before Stillkey is started from it.
  2. If an entry was added or Windows Boot Manager is no longer first, stop and use the rollback row "Firmware starts Stillkey by default".
  3. An internal second drive is used only after this check passes on this machine.
- With the Stillkey drive connected, two partitions hold a loader at the default path, because Windows normally keeps a copy of its boot manager at `\EFI\BOOT\BOOTX64.EFI`. Which one firmware's default-path search picks is not defined beyond "removable media, then fixed media" (UEFI §3.1.2). This is one more reason to keep Windows Boot Manager first in the stored order and to choose Stillkey by hand from the one-time boot menu.
- Later, if you want Stillkey in the persistent list, a named entry is added **through firmware setup**, placed after Windows Boot Manager. Microsoft documents no supported way to add a non-Windows entry with `bcdedit`, so we do not use one.
- An external USB drive is prepared from Windows with `tools/write-usb.ps1` under rule 8. Writing an internal second drive needs its own reviewed procedure in stage 5d, because bus type does not separate it from the Windows disk.

### 7.2 If there is only one drive

Options in order of preference:

1. Keep using removable media. No risk to Windows. Recommended until Stillkey has a reason to be installed.
2. Add a second drive (an external SSD is enough) and use 7.1.
3. Shrink the Windows partition to create a separate Stillkey EFI partition on the same disk. This changes the partition table, which Microsoft lists as a BitLocker recovery trigger, and it must not disturb the recovery partition that follows the Windows partition. It needs its own reviewed procedure, a full backup, and is outside this plan.

Placing Stillkey's files on the Windows EFI partition is not an option we will take (rule 2).

### 7.3 What must be unchanged afterwards

The before-and-after reports from `tools/host-report.ps1` must show identical: partition list and type GUIDs on the Windows disk; BitLocker protection status, protector types and IDs, and the PCR validation profile; Windows Boot Manager's firmware entry and its position; Windows RE status; Secure Boot state and the Microsoft certificates in `db` (the one accepted difference is the Stillkey entry added under route B). In addition, Windows must start with no recovery prompt.

The Windows EFI partition is never mounted for these reports (rule 1), so its files are not hashed. That they are unchanged rests on Stillkey having no way to write to a disk (rule 6) and on Windows starting with no recovery prompt.

## 8. Rollback

Prepared **before** any change:

- Recovery key located and its ID matched.
- A Windows recovery drive or installation media that matches the firmware's certificate state (media signed only with the 2023 certificate fail on firmware that lacks it, per KB5132953).
- A FAT32 USB stick, separate from the Stillkey stick, with `C:\Windows\Boot\EFI\SecureBootRecovery.efi` copied to `\EFI\BOOT\bootx64.efi` (KB5085046).
- The current Secure Boot variables saved to the project folder (section 5, rule 1).
- A current backup of your data.
- Photographs of the firmware setup pages we intend to touch.
- The firmware one-time boot menu key written down.
- No Windows or firmware update pending. If you agree to pause Windows updates for the test window, that is a setting change with its own row below.

| If this was done | Undo |
|---|---|
| USB stick written | Nothing on the PC changed. Reformat the stick |
| Secure Boot variables saved to `D:\OS\build`; recovery drive and SecureBootRecovery stick prepared | Nothing in firmware or on a Windows partition changed. Delete the `secureboot-*.bin` files when the project ends; reformat the sticks when they are no longer needed |
| BitLocker suspended | `Resume-BitLocker -MountPoint C:`; confirm `ProtectionStatus` is On and `manage-bde -protectors -get C: -Type TPM` shows the baseline PCR Validation Profile |
| Secure Boot switched off (route A) | 1. Recovery key located and ID matched. 2. `Suspend-BitLocker -MountPoint C: -RebootCount 0`, unless still suspended. 3. Switch Secure Boot back on in firmware setup with the plain switch. Do not use "restore factory keys", "install default keys", "clear keys" or "reset to Setup Mode". 4. Start Windows; confirm `Confirm-SecureBootUEFI` is True and `db` still contains the baseline certificates, "Windows UEFI CA 2023" among them on an updated PC. 5. `Resume-BitLocker -MountPoint C:` |
| Certificate or hash enrolled (route B) | 1. Recovery key located and ID matched. 2. `Suspend-BitLocker -MountPoint C: -RebootCount 0`, unless still suspended. 3. In firmware setup delete only the Stillkey entry from `db`. Do not use "restore factory keys", "install default keys", "clear keys" or "reset to Setup Mode". 4. Start Windows; confirm `Confirm-SecureBootUEFI` is True and `db` still contains the baseline certificates, "Windows UEFI CA 2023" among them on an updated PC. 5. `Resume-BitLocker -MountPoint C:` |
| USB boot switched on in firmware setup | Switch it back off; rule 5 applies |
| Second drive prepared | Disconnect it, or delete its partitions under rule 8. The Windows disk is untouched |
| Firmware boot entry added | Delete it in firmware setup (rule 5 applies); confirm Windows Boot Manager is first |
| Firmware starts Stillkey by default | Disconnect the drive; in firmware setup put Windows Boot Manager first and delete or disable the automatic entry; confirm with `bcdedit /enum firmware` |
| Windows Boot Manager entry missing | With the Stillkey drive disconnected: firmware "boot from file" › Windows EFI partition › `\EFI\Microsoft\Boot\bootmgfw.efi`; then recreate the entry in firmware setup, or use step 5 of "Windows does not start" |
| BitLocker recovery screen appears | Power off, remove Stillkey media, undo the last firmware change, start again. If it still asks, enter the 48-digit key whose ID matches. In Windows: `Suspend-BitLocker -MountPoint C: -RebootCount 0`, restart once, `Resume-BitLocker -MountPoint C:`; confirm the next start asks nothing |
| Firmware reports a Secure Boot violation for Windows | Keys were reset or removed. Do not repair Windows. Start the prepared SecureBootRecovery USB (KB5085046) or re-add the missing certificate in firmware setup |
| BitLocker now shows PCR 0, 2, 4, 11 instead of 7, 11 | With Secure Boot on: `Suspend-BitLocker -MountPoint C: -RebootCount 0`, restart, `Resume-BitLocker -MountPoint C:`; confirm with `manage-bde -protectors -get C: -Type TPM` |
| Hibernation turned off for testing | `powercfg /h on` |
| Windows updates paused | Settings › Windows Update › Resume updates |
| Windows does not start | 1. Power off; disconnect the Stillkey stick or drive. 2. Undo the last firmware change using its row above; nothing on a Windows disk was written, so this is the expected fix. 3. Firmware boot menu › Windows Boot Manager. 4. If a BitLocker or Secure Boot screen appears, use those rows. 5. Only if Windows' boot files are shown to be damaged, and only with the machine owner's explicit go-ahead, because this is the one exception to rule 2 (it writes the Windows EFI partition and firmware boot entries): Windows RE › Startup Repair; failing that, from matching recovery media with the Stillkey drive disconnected, run `diskpart` › `list vol` to find the Windows volume letter W, `manage-bde -unlock W: -RecoveryPassword <key>` (typed by you at the machine), then `bcdboot W:\Windows` (no `/s`, so the firmware entry is recreated and placed first). If firmware has revoked the 2011 Windows certificate, follow KB5025885 for `/bootex` |

Restoring factory keys is a last resort, allowed only if the inventory shows that `Get-SecureBootUEFI -Name dbDefault` contains "Windows UEFI CA 2023"; a variable reported as undefined counts as no. Microsoft's UEFI requirements say a reset to factory defaults re-establishes the maker's original keys and databases, and on firmware built before the 2023 certificates those originals do not contain them, so the reset can stop Windows starting under Secure Boot (KB5085046).

The rollback procedure is rehearsed once on the real machine before stage 5b begins, because 5b is the first stage that can change firmware state. The rehearsal: locate the recovery key and match its ID; suspend with `-RebootCount 0`, restart, resume, and confirm the baseline PCR validation profile; open the firmware boot menu; start recovery media and exit without changes.

## 9. Fast Startup and hibernation

With Fast Startup, "Shut down" saves the Windows kernel's memory image and the next start resumes it instead of cold booting. Windows then assumes its disks are exactly as it left them. Stillkey cannot write to any disk in this milestone, so it cannot break that assumption; nevertheless the procedure is to use **Restart**, or to turn hibernation off (`powercfg /h off`, elevated) for the test period, so that Windows always cold-boots after Stillkey has run. Turning hibernation off is a persistent Windows setting change: it needs the machine owner's go-ahead, is undone with `powercfg /h on`, and has a row in the rollback table. Using Restart needs no setting change and is preferred. The warning about other systems writing to a hibernated Windows volume comes from non-Microsoft documentation (the NTFS-3G manual); Microsoft's documentation does not cover the case.

## 10. Stage order

```
5a read-only report ──► you choose trust route and drive
      ──► 5-prep in the VM: local APIC timer, ACPI interrupt routing, loader failure screen and shutdown
      ──► 5c signing and verification proven in the VM (Secure Boot OVMF build, enrolled test keys)
      ──► recovery-key gate and rollback rehearsal on the machine
      ──► 5b removable-media boot on the machine
      ──► 5d separate-drive dual boot, machine-specific steps written and reviewed first
```
