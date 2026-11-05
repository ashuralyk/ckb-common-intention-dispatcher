use ckb_vm::{
    Bytes, DefaultCoreMachine, DefaultMachineBuilder, Error as VmError, Memory, Register,
    SparseMemory, SupportMachine, Syscalls, ISA_A, ISA_B, ISA_IMC, ISA_MOP,
};
use ckb_vm::registers::{A0, A1, A2, A7};
use std::sync::{Arc, Mutex};
use crate::{ExecutorError, CUSTOM_ERROR_START};

pub const SSRI_VM_VERSION: u64 = u64::MAX;
const MAX_CYCLES: u64 = 70_000_000;

/// Syscall numbers
const SYS_READ: u64 = 63;
const SYS_WRITE: u64 = 64;
const SYS_EXIT: u64 = 93;
const SYS_CKB_VM_VERSION: u64 = 2041;

/// SSRI stdio syscalls
struct SsriSyscalls {
    input: Arc<Mutex<Vec<u8>>>,
    output: Arc<Mutex<Vec<u8>>>,
    exit_code: Arc<Mutex<Option<i8>>>,
}

impl SsriSyscalls {
    fn new(input: Vec<u8>) -> Self {
        Self {
            input: Arc::new(Mutex::new(input)),
            output: Arc::new(Mutex::new(Vec::new())),
            exit_code: Arc::new(Mutex::new(None)),
        }
    }
}

impl<Mac: SupportMachine> Syscalls<Mac> for SsriSyscalls {
    fn initialize(&mut self, _machine: &mut Mac) -> Result<(), VmError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut Mac) -> Result<bool, VmError> {
        let syscall_num = machine.registers()[A7].to_u64();
        
        match syscall_num {
            SYS_READ => {
                let fd = machine.registers()[A0].to_u64();
                let buf_addr = machine.registers()[A1].to_u64();
                let len = machine.registers()[A2].to_u64() as usize;
                
                if fd == 0 {
                    // stdin
                    let mut input = self.input.lock().unwrap();
                    let to_read = std::cmp::min(len, input.len());
                    if to_read > 0 {
                        let data = input.drain(..to_read).collect::<Vec<_>>();
                        machine.memory_mut().store_bytes(buf_addr, &data)?;
                    }
                    machine.set_register(A0, Mac::REG::from_u64(to_read as u64));
                } else {
                    machine.set_register(A0, Mac::REG::from_u64(0));
                }
                Ok(true)
            }
            SYS_WRITE => {
                let fd = machine.registers()[A0].to_u64();
                let buf_addr = machine.registers()[A1].to_u64();
                let len = machine.registers()[A2].to_u64();
                
                if fd == 1 {
                    // stdout
                    let buf = machine.memory_mut().load_bytes(buf_addr, len)?;
                    self.output.lock().unwrap().extend_from_slice(&buf);
                    machine.set_register(A0, Mac::REG::from_u64(len));
                } else {
                    machine.set_register(A0, Mac::REG::from_u64(len));
                }
                Ok(true)
            }
            SYS_EXIT => {
                let code = machine.registers()[A0].to_i8();
                *self.exit_code.lock().unwrap() = Some(code);
                Err(VmError::InvalidEcall(SYS_EXIT))
            }
            SYS_CKB_VM_VERSION => {
                // Return u64::MAX to signal SSRI mode
                machine.set_register(A0, Mac::REG::from_u64(SSRI_VM_VERSION));
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

pub fn call_ssri_method(
    binary: &[u8],
    method_path: &str,
    input: &[u8],
) -> Result<Vec<u8>, ExecutorError> {
    let syscalls = SsriSyscalls::new(input.to_vec());
    let output_handle = syscalls.output.clone();
    let exit_handle = syscalls.exit_code.clone();
    
    let isa = ISA_IMC | ISA_A | ISA_B | ISA_MOP;
    let core_machine = DefaultCoreMachine::<u64, SparseMemory<u64>>::new(
        isa,
        SSRI_VM_VERSION as u32,
        MAX_CYCLES,
    );
    
    let mut machine = DefaultMachineBuilder::new(core_machine)
        .syscall(Box::new(syscalls))
        .build();
    
    // Load program with argv[0] = method path
    let code = Bytes::from(binary.to_vec());
    let args = vec![Bytes::from(method_path.as_bytes().to_vec())];
    machine.load_program(&code, args.into_iter().map(Ok)).map_err(|e| {
        ExecutorError::VmError(format!("failed to load program: {:?}", e))
    })?;
    
    // Run
    let result = machine.run();
    
    // Check result and exit code
    match result {
        Ok(_) | Err(VmError::InvalidEcall(SYS_EXIT)) => {
            // Program exited (either normally or via sys_exit)
            // Check exit code
            if let Some(code) = exit_handle.lock().unwrap().as_ref() {
                if *code != 0 {
                    if *code >= CUSTOM_ERROR_START {
                        return Err(ExecutorError::Incompatible(*code));
                    }
                    return Err(ExecutorError::VmError(format!("exit code {}", code)));
                }
            }
            // Exit code 0 or not set - success
            let output = output_handle.lock().unwrap().clone();
            Ok(output)
        }
        Err(e) => Err(ExecutorError::VmError(format!("VM error: {:?}", e))),
    }
}
