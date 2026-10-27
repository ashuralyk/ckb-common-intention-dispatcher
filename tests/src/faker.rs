//! FakeRpc universe for CID v0.1: always-success user locks, this CID binary,
//! real xUDT RISC-V, and a hash-mock Process cell (not SSRI).

use ckb_cinnabar_calculator::{
    address::{Address, AddressPayload},
    instruction::Instruction,
    operation::udt::XUDT_FAKENET_TX_HASH,
    re_exports::{
        ckb_hash::blake2b_256,
        ckb_types::{
            core::ScriptHashType,
            packed::{OutPoint, Script},
            prelude::{Builder, Entity, Pack, Unpack},
            H256,
        },
    },
    rpc::Network,
    simulation::{
        always_success_script, fake_header_view, fake_outpoint, AddFakeAlwaysSuccessCelldep,
        AddFakeContractCelldepByName, FakeRpcClient,
    },
    skeleton::{CellOutputEx, ScriptEx, TransactionSkeleton},
    types::{h256_to_hash, Hash256},
    TransactionCalculator,
};
use ckb_common_intention_dispatcher_calculator::{
    cid_lock, encode_cid_args, CidScript, Payload, CONTRACT_NAME, XUDT_CODE_HASH,
};

pub const CONTRACT_BIN: &str = "../build/release";
pub const XUDT_BIN: &str = "./binaries";
pub const CKB_CAPACITY: u64 = 200_000_000_000;
pub const XUDT_AMOUNT: u128 = 1_000;
pub const PACK_BLOCK: u64 = 1_000;
pub const USER_CELL_CAPACITY: u64 = 100_000_000_000;

pub fn user_script() -> Script {
    always_success_script(vec![])
}

pub fn dest_script() -> Script {
    always_success_script(vec![0xEE])
}

pub fn user_lock_cid() -> CidScript {
    script_to_cid(&user_script())
}

pub fn script_to_cid(script: &Script) -> CidScript {
    let mut code_hash = [0u8; 32];
    code_hash.copy_from_slice(script.code_hash().as_slice());
    let hash_type: u8 = script.hash_type().into();
    CidScript {
        code_hash,
        hash_type,
        args: script.args().raw_data().to_vec(),
    }
}

pub fn user_address() -> Address {
    Address::new(Network::Fake, AddressPayload::from(user_script()))
}

pub fn dest_lock() -> ScriptEx {
    dest_script().into()
}

pub fn user_lock() -> ScriptEx {
    user_script().into()
}

pub fn process_type_script() -> Script {
    Script::new_builder()
        .code_hash(H256([0xABu8; 32]).pack())
        .hash_type(ScriptHashType::Data1.into())
        .args(b"cid-process-v0".to_vec().pack())
        .build()
}

pub fn process_type_hash() -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash.copy_from_slice(process_type_script().calc_script_hash().as_slice());
    hash
}

pub fn cid_args(process_type_hash: [u8; 32], payload: &[u8]) -> Vec<u8> {
    encode_cid_args(
        process_type_hash,
        user_lock_cid(),
        Payload {
            data: payload.to_vec(),
        },
    )
}

pub fn xudt_type_script() -> ScriptEx {
    let issuer = user_script();
    let mut args = [0u8; 32].to_vec();
    args.copy_from_slice(issuer.calc_script_hash().as_slice());
    ScriptEx::new_code(XUDT_CODE_HASH, args)
}

pub fn outpoint_ids(out: &OutPoint) -> (Hash256, u32) {
    let tx_hash: H256 = out.tx_hash().unpack();
    (h256_to_hash(&tx_hash), out.index().unpack())
}

pub fn prepared_skeleton(rpc: &FakeRpcClient) -> TransactionSkeleton {
    let prepare = Instruction::<FakeRpcClient>::new(vec![
        Box::new(AddFakeAlwaysSuccessCelldep {}),
        Box::new(AddFakeContractCelldepByName {
            contract: CONTRACT_NAME.into(),
            type_id_args: None,
            contract_binary_path: CONTRACT_BIN.into(),
        }),
        Box::new(AddFakeContractCelldepByName {
            contract: "xudt".into(),
            type_id_args: None,
            contract_binary_path: XUDT_BIN.into(),
        }),
    ]);
    TransactionCalculator::default()
        .instruction(prepare)
        .new_skeleton(rpc)
        .expect("prepare skeleton")
        .0
}

pub fn seed_fakenet_xudt_cell(rpc: &mut FakeRpcClient) {
    let out = OutPoint::new(H256(XUDT_FAKENET_TX_HASH).pack(), 0);
    let cell = CellOutputEx::new_from_scripts(user_script(), None, vec![], Some(CKB_CAPACITY))
        .expect("fakenet xudt placeholder");
    rpc.insert_fake_cell(out, cell, None);
}

pub fn seed_user_cell(rpc: &mut FakeRpcClient, capacity: u64) {
    let cell = CellOutputEx::new_from_scripts(user_script(), None, vec![], Some(capacity))
        .expect("user cell");
    let header = fake_header_view(1, 1, 1);
    rpc.insert_fake_cell(fake_outpoint(), cell, Some(header));
}

pub fn seed_process_cell(rpc: &mut FakeRpcClient) -> OutPoint {
    let cell = CellOutputEx::new_from_scripts(
        user_script(),
        Some(process_type_script()),
        vec![],
        Some(CKB_CAPACITY),
    )
    .expect("process cell");
    let out = fake_outpoint();
    rpc.insert_fake_cell(out.clone(), cell, Some(fake_header_view(2, 2, 2)));
    out
}

pub fn seed_typed_dummy_cell(rpc: &mut FakeRpcClient) -> OutPoint {
    let dummy_type = Script::new_builder()
        .code_hash(H256([0xCDu8; 32]).pack())
        .hash_type(ScriptHashType::Data1.into())
        .args(b"not-process".to_vec().pack())
        .build();
    let cell = CellOutputEx::new_from_scripts(
        user_script(),
        Some(dummy_type),
        vec![],
        Some(CKB_CAPACITY),
    )
    .expect("dummy typed cell");
    let out = fake_outpoint();
    rpc.insert_fake_cell(out.clone(), cell, Some(fake_header_view(3, 3, 3)));
    out
}

pub fn seed_intention(
    rpc: &mut FakeRpcClient,
    skeleton: &TransactionSkeleton,
    args: Vec<u8>,
    capacity: u64,
    xudt: Option<(ScriptEx, u128)>,
    pack_block: u64,
) -> OutPoint {
    let lock = cid_lock(args).to_script(skeleton).expect("cid lock");
    let (type_script, data) = match xudt {
        Some((type_script, amount)) => (
            Some(type_script.to_script(skeleton).expect("xudt type")),
            amount.to_le_bytes().to_vec(),
        ),
        None => (None, vec![]),
    };
    let cell = CellOutputEx::new_from_scripts(lock, type_script, data, Some(capacity))
        .expect("intention cell");
    let out = fake_outpoint();
    rpc.insert_fake_cell(
        out.clone(),
        cell,
        Some(fake_header_view(pack_block, pack_block, pack_block)),
    );
    out
}

pub fn seed_header(rpc: &mut FakeRpcClient, block_number: u64) {
    rpc.insert_fake_header(fake_header_view(block_number, block_number, block_number));
}

pub fn seed_minimal_ckb_intention(
    rpc: &mut FakeRpcClient,
    skeleton: &TransactionSkeleton,
    args: Vec<u8>,
) -> OutPoint {
    let lock = cid_lock(args).to_script(skeleton).expect("cid lock");
    let cell = CellOutputEx::new_from_scripts(lock, None, vec![], None).expect("occupied-only");
    let out = fake_outpoint();
    rpc.insert_fake_cell(out.clone(), cell, Some(fake_header_view(PACK_BLOCK, 1, 1)));
    out
}

pub fn seed_typed_intention(
    rpc: &mut FakeRpcClient,
    skeleton: &TransactionSkeleton,
    args: Vec<u8>,
    type_script: Script,
) -> OutPoint {
    let lock = cid_lock(args).to_script(skeleton).expect("cid lock");
    let cell = CellOutputEx::new_from_scripts(lock, Some(type_script), vec![], Some(CKB_CAPACITY))
        .expect("typed intention");
    let out = fake_outpoint();
    rpc.insert_fake_cell(out.clone(), cell, Some(fake_header_view(PACK_BLOCK, 1, 1)));
    out
}

pub fn cid_code_hash() -> [u8; 32] {
    let bin = std::fs::read(format!("{CONTRACT_BIN}/{CONTRACT_NAME}")).expect("cid binary");
    blake2b_256(&bin)
}
