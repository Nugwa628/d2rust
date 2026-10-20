use super::*;

fn key() -> FrameKey {
    FrameKey {
        kind: 0,
        token: *b"QA  ",
        mode: *b"QW  ",
        component: 1,
        component_token: *b"QT  ",
        armor_class: *b"lit ",
        weapon_class: *b"hth ",
        direction: 8,
        frame: 2,
        sub_frame: 128,
    }
}

fn pack() -> FramePack {
    FramePack {
        input_hash: [31; 32],
        frames: [(
            key(),
            IndexFrame::new(2, 2, -3, 5, vec![0, 1, 128, 255]).unwrap(),
        )]
        .into(),
    }
}

#[test]
fn canonical_round_trip_preserves_all_key_fields_pixels_offsets_and_anchors() {
    let mut p = pack();
    for anchor in [FrameAnchor::Top, FrameAnchor::Bottom, FrameAnchor::TopDown] {
        p.frames.values_mut().next().unwrap().anchor = anchor;
        let bytes = p.encode().unwrap();
        assert_eq!(FramePack::decode(&bytes).unwrap(), p);
        assert_eq!(FramePack::decode(&bytes).unwrap().encode().unwrap(), bytes);
    }
    let base = key();
    let mut alternatives = Vec::new();
    let mut k = base.clone();
    k.kind = 1;
    alternatives.push(k);
    let mut k = base.clone();
    k.token = *b"QB  ";
    alternatives.push(k);
    let mut k = base.clone();
    k.mode = *b"QR  ";
    alternatives.push(k);
    let mut k = base.clone();
    k.component = 2;
    alternatives.push(k);
    let mut k = base.clone();
    k.component_token = *b"QL  ";
    alternatives.push(k);
    let mut k = base.clone();
    k.armor_class = *b"med ";
    alternatives.push(k);
    let mut k = base.clone();
    k.weapon_class = *b"bow ";
    alternatives.push(k);
    let mut k = base.clone();
    k.direction = 9;
    alternatives.push(k);
    let mut k = base.clone();
    k.frame = 3;
    alternatives.push(k);
    let mut k = base.clone();
    k.sub_frame = 129;
    alternatives.push(k);
    for k in alternatives {
        p.frames.insert(k, p.frames[&base].clone());
    }
    assert_eq!(p.frames.len(), 11);
    assert_eq!(FramePack::decode(&p.encode().unwrap()).unwrap(), p);
}

#[test]
fn malformed_versions_truncations_duplicates_and_resource_limits_are_rejected() {
    let bytes = pack().encode().unwrap();
    for end in 0..bytes.len() {
        assert!(FramePack::decode(&bytes[..end]).is_err());
    }
    let mut b = bytes.clone();
    b[8] = 2;
    assert_eq!(FramePack::decode(&b), Err(PackError::Version(2)));
    let mut b = bytes.clone();
    b.push(0);
    assert!(FramePack::decode(&b).is_err());
    let mut b = bytes.clone();
    b[42..46].copy_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&bytes[46..]);
    assert_eq!(
        FramePack::decode(&b),
        Err(PackError::Invalid("duplicate key"))
    );
    let mut b = bytes.clone();
    b[42..46].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(FramePack::decode(&b).is_err());
    for (offset, bad) in [(46, 5), (55, 16), (68, 64), (88, 3)] {
        let mut b = bytes.clone();
        b[offset] = bad;
        assert!(FramePack::decode(&b).is_err(), "offset {offset}");
    }
    let mut b = bytes;
    b[72..76].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(FramePack::decode(&b).is_err());
    let mut p = pack();
    p.frames.values_mut().next().unwrap().pixels.pop();
    assert!(p.encode().is_err());
    let mut p = pack();
    p.frames.values_mut().next().unwrap().width = 0;
    assert!(p.encode().is_err());
}

#[test]
fn seeded_mutation_fuzz_never_panics_and_every_accepted_pack_round_trips() {
    // Fixed seed is part of the regression; no art or external input is used.
    let seed = pack().encode().unwrap();
    let mut random = 0x7337_2026_1018u64;
    for trial in 0..4096 {
        let mut b = seed.clone();
        for _ in 0..1 + trial % 8 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let at = random as usize % b.len();
            b[at] ^= (random >> 32) as u8;
        }
        if let Ok(p) = FramePack::decode(&b) {
            assert_eq!(FramePack::decode(&p.encode().unwrap()).unwrap(), p);
        }
    }
}

#[test]
fn empty_pack_is_valid_and_tokens_and_placement_offsets_are_bounded() {
    let empty = FramePack {
        input_hash: [0; 32],
        frames: BTreeMap::new(),
    };
    assert_eq!(FramePack::decode(&empty.encode().unwrap()).unwrap(), empty);
    let bytes = pack().encode().unwrap();
    for token in [*b"../ ", [0; 4], [255; 4]] {
        let mut bad = bytes.clone();
        bad[47..51].copy_from_slice(&token);
        assert!(FramePack::decode(&bad).is_err());
    }
    for offset in [80, 84] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&i32::MIN.to_le_bytes());
        assert_eq!(
            FramePack::decode(&bad),
            Err(PackError::Invalid("picture offsets"))
        );
    }
    let mut p = pack();
    p.frames.values_mut().next().unwrap().x_off = i32::MAX;
    assert_eq!(p.encode(), Err(PackError::Invalid("picture offsets")));
}
