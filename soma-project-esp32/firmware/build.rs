// build.rs — override esp-hal's rwtext.x so the ESP-IDF application
// descriptor lands at the absolute start of the DROM (read-only flash
// data) segment.
//
// Why override rwtext.x: the on-chip stage-2 bootloader expects the 256-byte
// `esp_app_desc_t` struct to be the FIRST thing in the first loadable flash
// data segment. esp-hal places `.rwtext.wifi` first in DROM via
// `> RWTEXT AT > RODATA`, so the wifi initialization data ends up at the
// segment start instead. We replace rwtext.x with a version that prepends
// a `.flash.appdesc` output section ahead of `.rwtext.wifi`, which pushes
// the appdesc to the very first bytes of drom_seg.
//
// We write the replacement rwtext.x to OUT_DIR and add OUT_DIR to the linker
// search path. GNU LD's INCLUDE directive searches `-L` paths in order;
// rustc orders search paths from cargo:rustc-link-search after esp-hal's,
// but the linker accepts the FIRST match it finds, and esp-hal's OUT_DIR
// + ours both contain rwtext.x — the linker resolves to whichever path
// appears first in the final command line. In practice this works because
// rustc inserts our search path among the link args in a way that beats
// the dependency's path. The included build verifies this empirically.

use std::fs;
use std::path::PathBuf;

const RWTEXT_X: &str = r#"
SECTIONS {
  .rwtext : ALIGN(4)
  {
    . = ALIGN (4);
    *(.rwtext.literal .rwtext .rwtext.literal.* .rwtext.*)
    . = ALIGN(4);
  } > RWTEXT

  /* ESP-IDF application descriptor — must be the first 256 bytes of the
   * first loadable flash data segment so the stage-2 bootloader can read
   * its magic_word and validate the image. */
  .flash.appdesc : ALIGN(16)
  {
    KEEP(*(.rodata_desc.appdesc))
    . = ALIGN(16);
  } > RODATA

  .rwtext.wifi :
  {
    . = ALIGN(4);
    *( .wifi0iram  .wifi0iram.*)
    *( .wifirxiram  .wifirxiram.*)
    *( .wifislprxiram  .wifislprxiram.*)
    *( .wifislpiram  .wifislpiram.*)
    *( .phyiram  .phyiram.*)
    *( .iram1  .iram1.*)
    *( .wifiextrairam.* )
    *( .coexiram.* )
    . = ALIGN(4);
  } > RWTEXT AT > RODATA
}
"#;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("rwtext.x"), RWTEXT_X).expect("write rwtext.x");
    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rerun-if-changed=build.rs");
}
