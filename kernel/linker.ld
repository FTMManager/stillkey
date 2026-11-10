ENTRY(_start)
PHDRS {
  text   PT_LOAD FLAGS(5);   /* read + execute */
  rodata PT_LOAD FLAGS(4);   /* read */
  data   PT_LOAD FLAGS(6);   /* read + write */
}
SECTIONS {
  . = 0xFFFFFFFF80000000;
  __text_start = .;
  .text   : { *(.text .text.*) }                                                    :text
  __text_end = .;
  . = ALIGN(4K);
  __rodata_start = .;
  .rodata : { *(.rodata .rodata.*) *(.data.rel.ro .data.rel.ro.*) *(.got .got.*) }  :rodata
  __rodata_end = .;
  . = ALIGN(4K);
  __data_start = .;
  .data   : { *(.data .data.*) }                                                    :data
  .bss    : { *(.bss .bss.*) *(COMMON) }                                            :data
  __bss_end = .;
  /DISCARD/ : { *(.eh_frame*) *(.comment) *(.note*) }
}
