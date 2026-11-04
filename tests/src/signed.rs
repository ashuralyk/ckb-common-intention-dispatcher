//! Real genesis secp256k1 lock execution; FakeRpc supplies cells only.
use crate::faker::*;
use ckb_cinnabar_calculator::{
    address::{Address, AddressPayload},
    instruction::Instruction,
    operation::basic::{AddInputCellByOutPoint, AddOutputCell},
    re_exports::{
        ckb_hash::{blake2b_256, new_blake2b},
        ckb_types::{
            core::{Capacity, ScriptHashType},
            packed::{CellDep, Script},
            prelude::*,
        },
        secp256k1::{Message, PublicKey, Secp256k1, SecretKey},
    },
    rpc::Network,
    simulation::{fake_outpoint, FakeRpcClient, TransactionSimulator},
    skeleton::{CellDepEx, CellOutputEx, ChangeReceiver, TransactionSkeleton, WitnessEx},
    TransactionCalculator,
};
use ckb_common_intention_dispatcher_calculator::{
    create_intention, digest_incompatible, digest_ok, encode_cid_args, timeout_burn, Payload,
    TIMEOUT_BLOCKS,
};

fn key(byte: u8) -> SecretKey {
    SecretKey::from_slice(&[byte; 32]).unwrap()
}
fn lock(secret: &SecretKey) -> Script {
    let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), secret).serialize();
    Script::new_builder()
        .code_hash(ckb_system_scripts::CODE_HASH_SECP256K1_BLAKE160_SIGHASH_ALL.pack())
        .hash_type(ScriptHashType::Data)
        .args(blake2b_256(pubkey)[..20].to_vec().pack())
        .build()
}
fn secp_deps(skeleton: &mut TransactionSkeleton) {
    for name in ["secp256k1_blake160_sighash_all", "secp256k1_data"] {
        let data = ckb_system_scripts::BUNDLED_CELL
            .get(&format!("specs/cells/{name}"))
            .unwrap()
            .into_owned();
        let cell = CellOutputEx::new_from_scripts(user_script(), None, data.clone(), None).unwrap();
        skeleton.celldeps.push(CellDepEx::new(
            name.into(),
            CellDep::new_builder().out_point(fake_outpoint()).build(),
            cell.output,
            Some(data),
        ));
    }
}

// FakeRpc-compatible secp256k1 sighash signing for Data-hash locks.
// cinnabar's AddSecp256k1SighashSignatures is designed for mainnet/testnet Type-hash
// locks and does not recognize FakeRpc's ScriptHashType::Data + binary code_hash pattern.
fn sign_fake(skeleton: &mut TransactionSkeleton, secret: &SecretKey) {
    let script = lock(secret);
    let indices: Vec<usize> = skeleton
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(i, input)| (input.output.lock_script() == script).then_some(i))
        .collect();
    assert!(!indices.is_empty(), "no inputs match signing key");
    skeleton
        .witnesses
        .resize(skeleton.inputs.len(), WitnessEx::default());
    skeleton.witnesses[indices[0]] = WitnessEx::new(vec![0; 65], vec![], vec![]);
    let tx = skeleton.clone().into_transaction_view();
    let mut hasher = new_blake2b();
    hasher.update(tx.hash().as_slice());
    for i in indices
        .into_iter()
        .chain(skeleton.inputs.len()..skeleton.witnesses.len())
    {
        let witness = tx.witnesses().get(i).unwrap().raw_data();
        hasher.update(&(witness.len() as u64).to_le_bytes());
        hasher.update(&witness);
    }
    let mut digest = [0; 32];
    hasher.finalize(&mut digest);
    let sig = Secp256k1::new()
        .sign_ecdsa_recoverable(&Message::from_digest_slice(&digest).unwrap(), secret);
    let (id, compact) = sig.serialize_compact();
    let mut bytes = compact.to_vec();
    bytes.push(i32::from(id) as u8);
    let first = skeleton
        .inputs
        .iter()
        .position(|i| i.output.lock_script() == script)
        .unwrap();
    skeleton.witnesses[first].lock = bytes;
}
async fn verify(rpc: &FakeRpcClient, skeleton: TransactionSkeleton) -> bool {
    TransactionSimulator::default()
        .skeleton(skeleton)
        .link_cell_to_header(rpc.get_outpoint_to_headers())
        .async_verify(rpc, vec![], 50_000_000)
        .await
        .is_ok()
}

#[tokio::test]
async fn signed_paths_balance_change_and_bind_outputs() {
    for path in ["create", "digest", "incompatible", "timeout"] {
        let mut rpc = FakeRpcClient::default();
        seed_fakenet_xudt_cell(&mut rpc);
        let mut skeleton = prepared_skeleton(&rpc);
        secp_deps(&mut skeleton);
        let owner = key(1);
        let sponsor = key(2);
        let signer = if path == "incompatible" || path == "digest" {
            &sponsor
        } else {
            &owner
        };
        let payer_lock = lock(signer);
        let owner_lock = lock(&owner);
        let address = Address::new(Network::Fake, AddressPayload::from(payer_lock.clone()));
        let funding = fake_outpoint();
        rpc.insert_fake_cell(
            funding.clone(),
            CellOutputEx::new_from_scripts(
                payer_lock.clone(),
                None,
                vec![],
                Some(CKB_CAPACITY * 2),
            )
            .unwrap(),
            None,
        );
        let args = encode_cid_args(
            process_type_hash(),
            script_to_cid(&owner_lock),
            Payload { data: vec![] },
        );
        let intention = seed_intention(
            &mut rpc,
            &skeleton,
            args.clone(),
            CKB_CAPACITY,
            None,
            PACK_BLOCK,
        );
        let process = seed_process_cell(&mut rpc);
        if path == "digest" {
            // The Process lock belongs to the server, not the Intention owner.
            rpc.insert_fake_cell(
                process.clone(),
                CellOutputEx::new_from_scripts(
                    payer_lock.clone(),
                    Some(process_type_script()),
                    vec![],
                    Some(CKB_CAPACITY),
                )
                .unwrap(),
                None,
            );
        }
        let mut instructions = Vec::new();
        if path == "digest" || path == "incompatible" {
            let (tx_hash, index) = outpoint_ids(&funding);
            instructions.push(Instruction::new(vec![Box::new(AddInputCellByOutPoint {
                tx_hash,
                index,
                since: None,
            })]));
        }
        instructions.push(match path {
            "create" => create_intention(address, args, CKB_CAPACITY, None),
            "digest" => digest_ok(
                outpoint_ids(&intention),
                outpoint_ids(&process),
                Some(AddOutputCell {
                    lock_script: owner_lock.clone().into(),
                    type_script: None,
                    data: vec![],
                    capacity: CKB_CAPACITY / 2,
                    absolute_capacity: true,
                    type_id: false,
                }),
            ),
            "incompatible" => digest_incompatible(
                outpoint_ids(&intention),
                owner_lock.clone().into(),
                CKB_CAPACITY,
                None,
            ),
            _ => {
                seed_header(&mut rpc, PACK_BLOCK + TIMEOUT_BLOCKS);
                timeout_burn(
                    outpoint_ids(&intention),
                    address,
                    owner_lock.clone().into(),
                    CKB_CAPACITY,
                    None,
                    PACK_BLOCK + TIMEOUT_BLOCKS,
                )
            }
        });
        for ix in instructions {
            TransactionCalculator::default()
                .instruction(ix)
                .apply_skeleton(&rpc, &mut skeleton)
                .unwrap();
        }
        if path == "digest" {
            assert!(skeleton
                .inputs
                .iter()
                .all(|i| i.output.lock_script() != owner_lock));
        }
        // Explicit fee budget; a separate change output preserves exact refunds.
        let fee = 10_000;
        skeleton
            .balance(
                &rpc,
                Capacity::shannons(fee),
                payer_lock.clone().into(),
                ChangeReceiver::Script(payer_lock.clone().into()),
            )
            .unwrap();
        let total_in: u64 = skeleton
            .inputs
            .iter()
            .map(|i| i.output.capacity_shannons())
            .sum();
        let total_out: u64 = skeleton.outputs.iter().map(|o| o.capacity_shannons()).sum();
        assert_eq!(total_in - total_out, fee);
        assert_eq!(skeleton.outputs.last().unwrap().lock_script(), payer_lock);
        assert!(skeleton.outputs.last().unwrap().capacity_shannons() > 0);
        assert!(
            !verify(&rpc, skeleton.clone()).await,
            "{path}: unsigned must fail"
        );
        sign_fake(&mut skeleton, signer);
        assert!(skeleton.fee(&rpc, 0).unwrap().as_u64() <= fee);
        assert!(
            verify(&rpc, skeleton.clone()).await,
            "{path}: signed transaction must verify"
        );
        // Changing an output invalidates the funding signer (server for digest).
        skeleton.outputs[0].output = skeleton.outputs[0]
            .output
            .clone()
            .as_builder()
            .lock(user_script())
            .build();
        assert!(
            !verify(&rpc, skeleton).await,
            "{path}: output mutation must fail"
        );
    }
}
