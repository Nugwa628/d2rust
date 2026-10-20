//! S→C 0x7A PetAction as the server
//! builds it (`d2_sim::world::hirelings::pets::pet_action`, the encoder
//! of every 0x7A the server sends: the pet-add broadcast without seed and
//! name, and the hireling remove broadcasts) read by the client's handler
//! (the bridge's dispatch table).
//!
//! Both sides use owner @5 and pet @9 (`server-messages.tsv`, corrected
//! 2026-10-07). The test sends one add and one remove and checks that the
//! client's pet record holds the pet and owner the server meant.
use d2_client::bridge::dispatch::{Dispatch, Handle, Message};
use d2_client::bridge::output::Outputs;
use d2_client::bridge::world::{ClientWorld, ModelInputs, PET_HIRELING};
use d2_sim::world::hirelings::pets::{pet_action, ACTION_ADD, ACTION_REMOVE};

const PET: u32 = 0x21;
const OWNER: u32 = 0x05;
const CLASS: u16 = 0x14F;

fn receive(w: &mut ClientWorld, bytes: &[u8]) {
    let d = Dispatch::from_spec().expect("the spec's dispatch table");
    let entry = d.get(bytes[0]).expect("0x7A has a client handler");
    let Handle::General(handle) = entry.handle else {
        panic!("0x7A is a general handler");
    };
    let inputs = ModelInputs::default();
    let out = Outputs::default();
    let msg = Message {
        id: bytes[0],
        bytes,
        unit: None,
        inputs: &inputs,
        out: &out,
    };
    handle(w, &msg).expect("handled");
}

#[test]
fn server_0x7a_reaches_the_client_with_the_same_pet_and_owner() {
    let mut w = ClientWorld::default();
    // The add of a hireling (pet type 7) of class 0x14F.
    let add = pet_action(ACTION_ADD, PET_HIRELING, CLASS, PET, OWNER);
    assert_eq!(add.len(), 13);
    receive(&mut w, &add);
    assert_eq!(w.pets.len(), 1);
    let r = &w.pets[0];
    assert_eq!((r.pet_type, r.class), (PET_HIRELING, CLASS));
    assert_eq!(
        (r.pet, r.owner),
        (PET, OWNER),
        "the client read the server's pet GUID as the owner and the owner as the pet"
    );
    // The remove broadcast carries only the GUID: the client
    // must find the record by that pet GUID.
    let remove = pet_action(ACTION_REMOVE, 0, 0, PET, 0);
    receive(&mut w, &remove);
    assert!(
        w.pets.is_empty(),
        "the remove found no record: {:?}",
        w.pets
    );
}
