#![no_std]
#![no_main]

use core::arch::global_asm;

global_asm!(
    ".global _start",
    "_start:",
    "j _start"
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
