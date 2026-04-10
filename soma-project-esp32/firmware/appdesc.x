/* Place the ESP-IDF application descriptor at the very start of the DROM
 * (read-only data in flash) segment. The on-chip stage-2 bootloader expects
 * the 256-byte esp_app_desc_t struct to be the FIRST thing in the first
 * loadable flash segment, immediately after the image + segment headers.
 *
 * Without this, the section gets buried somewhere inside .rodata and the
 * bootloader reads garbage from where it thinks the descriptor lives,
 * producing errors like "Image requires efuse blk rev >= v237.62".
 *
 * INSERT BEFORE .rodata makes our section take precedence in the link
 * order — drom_seg starts with our 256 bytes, then normal .rodata follows.
 */
SECTIONS {
  .rodata_desc.appdesc : ALIGN(16)
  {
    _esp_app_desc_start = ABSOLUTE(.);
    KEEP(*(.rodata_desc.appdesc))
    . = ALIGN(16);
  } > RODATA
}
INSERT BEFORE .rodata;
