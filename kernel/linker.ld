ENTRY(_start)
PHDRS {
  text   PT_LOAD FLAGS(5);   /* read + execute */
  rodata PT_LOAD FLAGS(4);   /* read */
  data   PT_LOAD FLAGS(6);   /* read + write */
}
SECTIONS {
  . = 0xFFFFFFFF80000000;
  .text   : { *(.text .text.*) }                                                    :text
  . = ALIGN(4K);
  .rodata : { *(.rodata .rodata.*) *(.data.rel.ro .data.rel.ro.*) *(.got .got.*) }  :rodata
  . = ALIGN(4K);
  .data   : { *(.data .data.*) }                                                    :data
  .bss    : { *(.bss .bss.*) *(COMMON) }                                            :data
  /DISCARD/ : { *(.eh_frame*) *(.comment) *(.note*) }
}
