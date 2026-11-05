#![no_std]
#![cfg_attr(not(test), no_main)]

mod ssri;
mod verify;

use ckb_std::env;

pub enum Error {
    SSRIMethodNotFound,
    SSRIMethodArgsInvalid,
    VerifyError(i8),
}

impl From<ckb_std::error::SysError> for Error {
    fn from(_: ckb_std::error::SysError) -> Self {
        Error::SSRIMethodArgsInvalid
    }
}

impl From<Error> for i8 {
    fn from(e: Error) -> i8 {
        match e {
            Error::SSRIMethodNotFound => 10,
            Error::SSRIMethodArgsInvalid => 11,
            Error::VerifyError(code) => code,
        }
    }
}

#[cfg(not(test))]
fn ckb_std_main_inner() -> Result<(), Error> {
    use ckb_ssri_std::ssri_methods;
    
    // Check if this is an SSRI call (non-empty argv means SSRI mode)
    if !env::argv().is_empty() {
        let result = ssri_methods! {
            argv: env::argv(),
            invalid_method: Error::SSRIMethodNotFound,
            invalid_args: Error::SSRIMethodArgsInvalid,
            "Process.batch" => ssri::batch_handler(),
            "Process.assemble" => ssri::assemble_handler(),
        };
        
        match result {
            Ok(output) => {
                // Write output to stdout
                ckb_std::syscalls::write(1, &output)
                    .map_err(|_| Error::VerifyError(12))?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    } else {
        // Otherwise run on-chain verification
        verify::verify_process()
    }
}

#[cfg(not(test))]
fn ckb_std_main() -> i8 {
    match ckb_std_main_inner() {
        Ok(()) => 0,
        Err(e) => i8::from(e),
    }
}

#[cfg(not(test))]
ckb_std::entry!(ckb_std_main);

#[cfg(not(test))]
ckb_std::default_alloc!();
