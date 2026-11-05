use crate::{ExecutorError, CUSTOM_ERROR_START};
use ckb_hash::blake2b_256;
use ckb_vm::cost_model::estimate_cycles;
use ckb_vm::registers::{A0, A1, A2, A7};
use ckb_vm::{
    Bytes, CoreMachine, DefaultCoreMachine, DefaultMachineBuilder, Error as VmError, Memory,
    Register, SparseMemory, SupportMachine, Syscalls, ISA_A, ISA_B, ISA_IMC, ISA_MOP,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const SSRI_VM_VERSION: u64 = u64::MAX;
const MAX_CYCLES: u64 = 70_000_000;
/// Cap captured stdout / pipe buffers so a looping guest cannot grow host RAM.
const MAX_HOST_BUFFER: usize = 1024 * 1024;

/// Syscall numbers (Linux-style, for fixture tests)
const SYS_READ: u64 = 63;
const SYS_WRITE: u64 = 64;
const SYS_EXIT: u64 = 93;
const SYS_CKB_VM_VERSION: u64 = 2041;

/// Syscall numbers (ckb-std 1.1.0 from ckb_constants.rs)
const SYS_SPAWN: u64 = 2049;
const SYS_LOAD_CELL_DATA: u64 = 2081;
const SYS_LOAD_BLOCK_EXTENSION: u64 = 2137;
const SYS_DEBUG: u64 = 2177;
const SYS_PIPE: u64 = 2604;
const SYS_CKB_WRITE: u64 = 2605;
const SYS_CKB_READ: u64 = 2606;
const SYS_INHERITED_FDS: u64 = 2607;
const SYS_CLOSE: u64 = 2608;

/// Pipe pair (read_fd, write_fd)
#[derive(Clone, Debug)]
struct Pipe {
    buffer: Vec<u8>,
}

/// SSRI stdio syscalls
struct SsriSyscalls {
    input: Arc<Mutex<Vec<u8>>>,
    output: Arc<Mutex<Vec<u8>>>,
    exit_code: Arc<Mutex<Option<i8>>>,
    pipes: Arc<Mutex<HashMap<u64, Pipe>>>,
    next_fd: Arc<Mutex<u64>>,
}

impl SsriSyscalls {
    fn new(input: Vec<u8>) -> Self {
        Self {
            input: Arc::new(Mutex::new(input)),
            output: Arc::new(Mutex::new(Vec::new())),
            exit_code: Arc::new(Mutex::new(None)),
            pipes: Arc::new(Mutex::new(HashMap::new())),
            next_fd: Arc::new(Mutex::new(3)), // Start at fd 3 (0=stdin, 1=stdout, 2=stderr)
        }
    }
}

fn append_capped(buf: &mut Vec<u8>, data: &[u8]) -> Result<(), VmError> {
    let new_len = buf.len().saturating_add(data.len());
    if new_len > MAX_HOST_BUFFER {
        return Err(VmError::External("host buffer cap exceeded".into()));
    }
    buf.try_reserve(data.len())
        .map_err(|_| VmError::External("host buffer reserve failed".into()))?;
    buf.extend_from_slice(data);
    Ok(())
}

fn load_guest_bytes<Mac: SupportMachine>(
    machine: &mut Mac,
    addr: u64,
    len: u64,
) -> Result<Bytes, VmError> {
    if len as usize > MAX_HOST_BUFFER {
        return Err(VmError::External(
            "guest write exceeds host buffer cap".into(),
        ));
    }
    machine.memory_mut().load_bytes(addr, len)
}

impl<Mac: SupportMachine> Syscalls<Mac> for SsriSyscalls {
    fn initialize(&mut self, _machine: &mut Mac) -> Result<(), VmError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut Mac) -> Result<bool, VmError> {
        let syscall_num = machine.registers()[A7].to_u64();

        match syscall_num {
            // Linux-style read (for fixture tests)
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
            // Linux-style write (for fixture tests)
            SYS_WRITE => {
                let fd = machine.registers()[A0].to_u64();
                let buf_addr = machine.registers()[A1].to_u64();
                let len = machine.registers()[A2].to_u64();

                if fd == 1 {
                    let buf = load_guest_bytes(machine, buf_addr, len)?;
                    append_capped(&mut self.output.lock().unwrap(), &buf)?;
                    machine.set_register(A0, Mac::REG::from_u64(len));
                } else {
                    machine.set_register(A0, Mac::REG::from_u64(len));
                }
                Ok(true)
            }
            // ckb-std pipe(): create pipe pair, write [read_fd, write_fd] to pointer
            SYS_PIPE => {
                let fds_ptr = machine.registers()[A0].to_u64();

                let read_fd = *self.next_fd.lock().unwrap();
                let write_fd = read_fd + 1;
                *self.next_fd.lock().unwrap() = write_fd + 1;

                // Store pipe for write_fd (writes to this fd append to pipe buffer)
                self.pipes
                    .lock()
                    .unwrap()
                    .insert(write_fd, Pipe { buffer: Vec::new() });

                // Write [read_fd, write_fd] as two u64s at fds_ptr
                let mut fds_bytes = Vec::with_capacity(16);
                fds_bytes.extend_from_slice(&read_fd.to_le_bytes());
                fds_bytes.extend_from_slice(&write_fd.to_le_bytes());
                machine.memory_mut().store_bytes(fds_ptr, &fds_bytes)?;

                // Success: a0 = 0
                machine.set_register(A0, Mac::REG::from_u64(0));
                Ok(true)
            }
            // ckb-std write(fd, buf_ptr, len_ptr)
            // Success: a0=0, *len_ptr=bytes_written; Fail: a0!=0
            SYS_CKB_WRITE => {
                let fd = machine.registers()[A0].to_u64();
                let buf_ptr = machine.registers()[A1].to_u64();
                let len_ptr = machine.registers()[A2].to_u64();

                // Read length from pointer
                let len_bytes = machine.memory_mut().load_bytes(len_ptr, 8)?;
                let len = u64::from_le_bytes(len_bytes.as_ref().try_into().unwrap()) as usize;

                let data = load_guest_bytes(machine, buf_ptr, len as u64)?;

                if let Some(pipe) = self.pipes.lock().unwrap().get_mut(&fd) {
                    append_capped(&mut pipe.buffer, &data)?;
                    append_capped(&mut self.output.lock().unwrap(), &data)?;
                } else if fd == 1 {
                    append_capped(&mut self.output.lock().unwrap(), &data)?;
                }

                // Write back length
                machine
                    .memory_mut()
                    .store_bytes(len_ptr, &(len as u64).to_le_bytes())?;

                // Success: a0 = 0
                machine.set_register(A0, Mac::REG::from_u64(0));
                Ok(true)
            }
            // ckb-std read(fd, buf_ptr, len_ptr)
            // Success: a0=0, *len_ptr=bytes_read; Fail: a0!=0
            SYS_CKB_READ => {
                let fd = machine.registers()[A0].to_u64();
                let buf_ptr = machine.registers()[A1].to_u64();
                let len_ptr = machine.registers()[A2].to_u64();

                // Read length from pointer
                let len_bytes = machine.memory_mut().load_bytes(len_ptr, 8)?;
                let len = u64::from_le_bytes(len_bytes.as_ref().try_into().unwrap()) as usize;

                let bytes_read = if fd == 0 {
                    // stdin
                    let mut input = self.input.lock().unwrap();
                    let to_read = std::cmp::min(len, input.len());
                    if to_read > 0 {
                        let data = input.drain(..to_read).collect::<Vec<_>>();
                        machine.memory_mut().store_bytes(buf_ptr, &data)?;
                    }
                    to_read
                } else {
                    0 // Other fds not supported
                };

                // Write back length
                machine
                    .memory_mut()
                    .store_bytes(len_ptr, &(bytes_read as u64).to_le_bytes())?;

                // Success: a0 = 0
                machine.set_register(A0, Mac::REG::from_u64(0));
                Ok(true)
            }
            // ckb-std close(fd)
            SYS_CLOSE => {
                let fd = machine.registers()[A0].to_u64();
                self.pipes.lock().unwrap().remove(&fd);

                // Success: a0 = 0
                machine.set_register(A0, Mac::REG::from_u64(0));
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
            // Stub implementations for syscalls we don't need for SSRI but might be called
            SYS_SPAWN => {
                // Spawn not supported in offline mode
                machine.set_register(A0, Mac::REG::from_u64(1)); // error
                Ok(true)
            }
            SYS_LOAD_CELL_DATA => {
                // Cell data loading not supported in offline mode
                machine.set_register(A0, Mac::REG::from_u64(1)); // error
                Ok(true)
            }
            SYS_LOAD_BLOCK_EXTENSION => {
                // Block extension not available in offline mode
                machine.set_register(A0, Mac::REG::from_u64(1)); // error
                Ok(true)
            }
            SYS_DEBUG => {
                // debug! macro - no-op success
                machine.set_register(A0, Mac::REG::from_u64(0));
                Ok(true)
            }
            SYS_INHERITED_FDS => {
                // Return no inherited fds
                machine.set_register(A0, Mac::REG::from_u64(0)); // success, 0 fds
                Ok(true)
            }
            _ if (2041..=3000).contains(&syscall_num) => {
                // CKB syscall range - return error for unsupported ones
                machine.set_register(A0, Mac::REG::from_u64(1)); // error
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// Convert method path to hex-encoded method ID for argv[0]
/// ssri_methods! expects argv[0] to be hex of u64::from_le_bytes(blake2b_256(path)[0..8])
fn ssri_method_argv0(method_path: &str) -> String {
    let hash = blake2b_256(method_path.as_bytes());
    let id_bytes = &hash[0..8];
    hex::encode(id_bytes)
}

pub fn call_ssri_method(
    binary: &[u8],
    method_path: &str,
    input: &[u8],
) -> Result<Vec<u8>, ExecutorError> {
    call_ssri_method_with_cycles(binary, method_path, input, MAX_CYCLES)
}

pub fn call_ssri_method_with_cycles(
    binary: &[u8],
    method_path: &str,
    input: &[u8],
    max_cycles: u64,
) -> Result<Vec<u8>, ExecutorError> {
    let syscalls = SsriSyscalls::new(input.to_vec());
    let output_handle = syscalls.output.clone();
    let exit_handle = syscalls.exit_code.clone();

    let isa = ISA_IMC | ISA_A | ISA_B | ISA_MOP;
    let core_machine =
        DefaultCoreMachine::<u64, SparseMemory<u64>>::new(isa, SSRI_VM_VERSION as u32, max_cycles);

    let mut machine = DefaultMachineBuilder::new(core_machine)
        .instruction_cycle_func(Box::new(estimate_cycles))
        .syscall(Box::new(syscalls))
        .build();

    // Load program with argv[0] = hex-encoded method ID
    let code = Bytes::from(binary.to_vec());
    let method_id_hex = ssri_method_argv0(method_path);
    let args = vec![Bytes::from(method_id_hex.as_bytes().to_vec())];
    machine
        .load_program(&code, args.into_iter().map(Ok))
        .map_err(|e| ExecutorError::VmError(format!("failed to load program: {:?}", e)))?;

    // Run
    let result = machine.run();

    // Check if exit was called via syscall, or get return value from a0
    let final_code = if let Some(code) = exit_handle.lock().unwrap().as_ref() {
        *code
    } else {
        // VM completed naturally - check register a0 for return value
        machine.registers()[A0].to_i8()
    };

    // Check result and exit code
    match result {
        Err(VmError::CyclesExceeded) => Err(ExecutorError::CyclesExceeded),
        Ok(_) | Err(VmError::InvalidEcall(SYS_EXIT)) => {
            // Program exited (either via sys_exit or naturally)
            // Use final_code which is either from SYS_EXIT or from register a0
            if final_code != 0 {
                if final_code >= CUSTOM_ERROR_START {
                    return Err(ExecutorError::Incompatible(final_code));
                }
                return Err(ExecutorError::VmError(format!("exit code {}", final_code)));
            }
            // Exit code 0 - success
            let output = output_handle.lock().unwrap().clone();
            Ok(output)
        }
        Err(e) => Err(ExecutorError::VmError(format!("VM error: {:?}", e))),
    }
}
