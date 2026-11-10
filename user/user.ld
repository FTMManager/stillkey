/* Shared by every ring-3 program (docs/10 section 4). Static, at 4 MiB, with
   one segment per permission: the kernel maps each with exactly its flags
   and refuses a file that asks for writable code. */
ENTRY(_start)
PHDRS {
  text   PT_LOAD FLAGS(5);   /* read + execute */
  rodata PT_LOAD FLAGS(4);   /* read */
  data   PT_LOAD FLAGS(6);   /* read + write */
}
SECTIONS {
  . = 0x400000;
  .text   : { *(.text .text.*) }                                                             :text
  . = ALIGN(4K);
  /* The leading word keeps a segment from being empty, which the validator
     refuses (rule V5). */
  .rodata : { QUAD(0) *(.rodata .rodata.*) *(.data.rel.ro .data.rel.ro.*) *(.got .got.*) }   :rodata
  . = ALIGN(4K);
  .data   : { QUAD(0) *(.data .data.*) }                                                     :data
  .bss    : { *(.bss .bss.*) *(COMMON) }                                                     :data
  /DISCARD/ : { *(.eh_frame*) *(.comment) *(.note*) }
}
