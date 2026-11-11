# Stillkey: design and build plan

Stillkey is an original desktop operating system: a private workspace with a built-in Workbench that shows what every app is doing and what it can access. This folder is its design and build plan. Milestone 1 (boot) is implemented and passes its tests in a virtual machine ([07 §0](07-milestone-1-spec.md#0-status-2026-11-10-implemented)). Milestone 2 (run and observe) has a specification ([10](10-milestone-2-spec.md)); all three of its checkpoints are implemented and pass their tests: one ring-3 program that prints and exits under a running timer ([10 §11.2](10-milestone-2-spec.md#112-checkpoint-1-result-2026-11-10)), then several programs under a pre-emptive scheduler, a crash that ends only the program that caused it, the inspector, and the kernel log on the screen ([10 §11.3](10-milestone-2-spec.md#113-checkpoint-2-result-2026-11-10)). The third adds seventeen programs that each try something a program may not do, an image the kernel refuses to start, and the acceptance runs: twenty passes of every test in dev and release builds, under KVM and under software emulation ([10 §11.4](10-milestone-2-spec.md#114-checkpoint-3-result-2026-11-11)). An independent review on 2026-11-11 found one way for any program to stop the kernel and several checks that could not fail; all are fixed ([10 §11.5](10-milestone-2-spec.md#115-independent-review-and-fixes-2026-11-11)). Milestones 1 and 2 are accepted. Milestones 3 to 5 are plans. Milestone 4 has a second stage, a desktop assistant that acts only through grants the person makes ([06](06-milestones.md#stage-4b--assistant), [04 §11](04-security-and-permissions.md#11-delegation-to-an-assistant-stage-4b)). Milestone 3 (grant and revoke) has a specification ([11](11-milestone-3-spec.md)) and is not started.

Read in this order:

| # | Document | Answers |
|---|---|---|
| 01 | [Overview and architecture](01-overview-and-architecture.md) | Requirements, assumptions and design decisions; the boot-to-desktop diagram; repository layout; workflow; dependencies |
| 02 | [Boot and handoff](02-boot-and-handoff.md) | How the loader finds, checks and loads the kernel; `BootInfo` v1; the `ExitBootServices` sequence; the entry contract |
| 03 | [Kernel and interfaces](03-kernel-and-interfaces.md) | Kernel subsystems, where drivers run, assembly and `unsafe`, system calls, input, graphics, storage |
| 04 | [Security and permissions](04-security-and-permissions.md) | Threat model, handles and grants, revocation, what the Workbench may see, what each milestone proves |
| 05 | [Interface design](05-ui-design.md) | Visual identity, the three screens, flow, rendering stages, accessibility |
| 06 | [Milestones](06-milestones.md) | Dependencies, deliverables, pass criteria and failure points for all five |
| 07 | [Milestone 1 specification](07-milestone-1-spec.md) | Files, build, VM command, expected serial output, tests, debugging |
| 08 | [Hardware and dual boot](08-hardware-and-dual-boot.md) | Read-only checks, removable-media testing, Secure Boot routes, dual-boot design, rollback. Plan only |
| 09 | [References](09-references.md) | The specifications and documentation behind the claims |
| 10 | [Milestone 2 specification](10-milestone-2-spec.md) | Crates, `BOOTFS.IMG`, boot steps and log lines, system calls, programs and probes, tests, implementation order. Implemented and accepted |
| 11 | [Milestone 3 specification](11-milestone-3-spec.md) | Objects and handles, channels, files, grants, the audit log, the console, `init`, `permd`, the shell, programs and probes, tests, implementation order. Not started |

Conventions: **Confirmed** means a stated requirement or a cited specification. **Assumption** (A-nn) means not yet verified, with the check named. **Decision** (D-…) means a choice made in this design, recorded with the alternative considered; the full list is in [01 §3](01-overview-and-architecture.md#3-design-decisions). Paths such as `D:\OS` and `/mnt/d/OS`, and the figures marked as observed, are those of the development machine (Windows 11 with WSL2); the wrapper `tools/sk.ps1` names that path, so a checkout elsewhere changes it there.

Safety boundary: Milestones 1–4 run only inside WSL2 and QEMU. Nothing touches a physical disk, firmware settings, Secure Boot, BitLocker or the Windows boot configuration until Milestone 5, and then only after the read-only inventory in 08 and the machine owner's explicit go-ahead for each step.
