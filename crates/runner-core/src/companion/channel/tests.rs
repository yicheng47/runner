use super::*;
use crate::remote::{encode_key, HostKey};
use serde_json::{json, Value};

pub(super) fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}
fn keys(n: u8) -> NoiseKeys<'static> {
    match n {
        1 => NoiseKeys {
            private: &[1; 32],
            fixed_ephemeral: Some(&[3; 32]),
        },
        2 => NoiseKeys {
            private: &[2; 32],
            fixed_ephemeral: Some(&[4; 32]),
        },
        _ => unreachable!(),
    }
}
fn pair(pin: &str) -> (BrowserPair, HostPair, Vec<u8>, Vec<u8>) {
    let (browser, first) = BrowserPair::start(keys(1), pin, [5; 16], [6; 32]).unwrap();
    let (host, reply) = HostPair::start(keys(2), "012345", &first, [7; 32]).unwrap();
    (browser, host, first, reply)
}

fn encrypt_record(transport: &mut Transport, record: &[u8]) -> Vec<u8> {
    let mut encrypted = vec![0; record.len() + 16];
    transport.write_message(record, &mut encrypted).unwrap();
    frame(Kind::Data, &encrypted)
}
fn decrypt_record(transport: &mut Transport, frame: &[u8]) -> Vec<u8> {
    let mut plain = vec![0; frame.len()];
    let n = transport.read_message(&frame[1..], &mut plain).unwrap();
    plain.truncate(n);
    plain
}

fn transcript() -> Value {
    let (b, h, pair_frame, reply) = pair("012345");
    let (bt, finish, host_key) = b.finish(&reply, "Chrome on Windows").unwrap();
    let (ht, device) = h.finish(&finish).unwrap();
    let mut browser = bt;
    let mut host = Records::new(ht);
    let hello = br#"{"type":"hello","versions":[1],"client":"web","build":"vector"}"#;
    let first_part = [&[0], &hello[..31]].concat();
    let last_part = [&[1], &hello[31..]].concat();
    let part_frame = encrypt_record(&mut browser, &first_part);
    assert!(host.receive(&part_frame).unwrap().is_none());
    let hello_frame = encrypt_record(&mut browser, &last_part);
    assert_eq!(host.receive(&hello_frame).unwrap().unwrap(), hello);
    host.handled();
    let credit = host.next_frame().unwrap().unwrap();
    assert_eq!(
        decrypt_record(&mut browser, &credit),
        [2, 0, 0, 0, hello.len() as u8]
    );
    let welcome = br#"{"type":"welcome","version":1,"name":"Mac Studio","runnerVersion":"0.16.7","platform":"macos","features":[]}"#;
    host.queue(welcome.to_vec()).unwrap();
    let w = host.next_frame().unwrap().unwrap();
    assert_eq!(
        decrypt_record(&mut browser, &w),
        [&[1], welcome.as_slice()].concat()
    );
    let bc = encrypt_record(&mut browser, &[2, 0, 0, 0, welcome.len() as u8]);
    host.receive(&bc).unwrap();
    let (b, connect_frame) = BrowserConnect::start(keys(1), &host_key).unwrap();
    let (_, accept, public) =
        connect(keys(2), &connect_frame, |key| key == device.public_key).unwrap();
    assert_eq!(public, device.public_key);
    let _ = b.accept(&accept).unwrap();
    let (b, h, wp, wr) = pair("999999");
    let (_, wf, _) = b.finish(&wr, "Chrome on Windows").unwrap();
    assert!(matches!(h.finish(&wf), Err(Reject::WrongPin)));
    let unknown = connect(keys(2), &connect_frame, |_| false).unwrap_err();
    let cpace = Cpace::new(b"012345", PAIR_PROLOGUE, &[5; 16], [6; 32]).unwrap();
    let isk = cpace
        .isk(&reply[1..33].try_into().unwrap(), &[5; 16], true, b"", b"")
        .unwrap();
    let rejects: serde_json::Map<String, Value> = [
        Reject::NoPin,
        Reject::PinLocked,
        Reject::WrongPin,
        Reject::UnknownDevice,
        Reject::Busy,
        Reject::BadHandshake,
    ]
    .into_iter()
    .map(|r| (r.reason().into(), json!(hex(&r.frame()))))
    .collect();
    json!({
        "format": 1,
        "suites": {"pair":"Noise_XXpsk3_25519_ChaChaPoly_SHA256","connect":"Noise_IK_25519_ChaChaPoly_SHA256","pairPrologue":hex(PAIR_PROLOGUE),"connectPrologue":hex(b"runner-connect-v1"),"ci":hex(PAIR_PROLOGUE),"ada":"","adb":""},
        "isk":hex(&isk), "psk":hex(&isk[..32]),
        "rejects":rejects,
        "route":{"channel":17,"pair":hex(&wrap(17,&pair_frame))},
        "inputs": {"browserStatic":hex(&[1;32]), "hostStatic":hex(&[2;32]), "browserEphemeral":hex(&[3;32]), "hostEphemeral":hex(&[4;32]), "sid":hex(&[5;16]), "browserScalar":hex(&[6;32]), "hostScalar":hex(&[7;32]), "pin":"012345", "name":"Chrome on Windows", "wrongPin":"999999"},
        "pair": [hex(&pair_frame),hex(&reply),hex(&finish)],
        "records": [json!({"from":"browser","plaintext":hex(&first_part),"frame":hex(&part_frame)}),json!({"from":"browser","plaintext":hex(&last_part),"frame":hex(&hello_frame)}),json!({"from":"host","plaintext":hex(&[2,0,0,0,hello.len() as u8]),"frame":hex(&credit)}),json!({"from":"host","plaintext":hex(&[&[1],welcome.as_slice()].concat()),"frame":hex(&w)}),json!({"from":"browser","plaintext":hex(&[2,0,0,0,welcome.len() as u8]),"frame":hex(&bc)})],
        "connect": [hex(&connect_frame),hex(&accept)],

        "wrongPin": [hex(&wp),hex(&wr),hex(&wf),hex(&Reject::WrongPin.frame())],
        "unknownDevice": [hex(&connect_frame),hex(&unknown.frame())],
        "hostPublic":hex(&host_key), "browserPublic":hex(&device.public_key)
    })
}

#[test]
fn golden_channel_transcripts() {
    let actual = transcript();
    if std::env::var_os("UPDATE_CHANNEL_VECTORS").is_some() {
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/companion/channel/vectors/channel.json"),
            serde_json::to_string_pretty(&actual).unwrap() + "\n",
        )
        .unwrap();
    } else {
        assert_eq!(
            actual,
            serde_json::from_str::<Value>(include_str!("vectors/channel.json")).unwrap()
        );
    }
}

fn transports() -> (snow::TransportState, snow::TransportState) {
    let (b, h, _, reply) = pair("012345");
    let (bt, finish, _) = b.finish(&reply, "test").unwrap();
    let (ht, _) = h.finish(&finish).unwrap();
    (bt, ht)
}

#[test]
fn framing_lengths_order_points_and_confirmation_failures() {
    let (_, _, first, _) = pair("012345");
    for n in [0, 1, 96, 98] {
        assert!(HostPair::start(keys(2), "012345", &vec![3; n], [7; 32]).is_err());
    }
    for invalid in [[0; 32], [255; 32]] {
        let mut bytes = first.clone();
        bytes[17..49].copy_from_slice(&invalid);
        assert!(HostPair::start(keys(2), "012345", &bytes, [7; 32]).is_err());
    }
    for (pin, tamper, expected) in [
        ("999999", false, Reject::WrongPin),
        ("012345", true, Reject::WrongPin),
    ] {
        let (b, h, _, r) = pair(pin);
        let (_, mut f, _) = b.finish(&r, "test").unwrap();
        if tamper {
            *f.last_mut().unwrap() ^= 1;
        }
        assert!(matches!(h.finish(&f), Err(e) if e == expected));
    }
    for input in [vec![5; 64], vec![6; 100], vec![5; MAX_FRAME + 1]] {
        let (_, h, _, _) = pair("012345");
        assert!(matches!(h.finish(&input), Err(Reject::BadHandshake)));
    }
    for name in ["", &"a".repeat(65)] {
        let (b, h, _, r) = pair("012345");
        let (_, f, _) = b.finish(&r, name).unwrap();
        assert!(matches!(h.finish(&f), Err(Reject::BadHandshake)));
    }
    let (b, h, _, r) = pair("012345");
    let (_, f, _) = b.finish(&r, "a\n\u{7f}b").unwrap();
    assert_eq!(h.finish(&f).unwrap().1.name, "ab");
    assert!(unwrap(&[0; 4]).is_none());
    assert!(unwrap(&[0; 5]).is_none());
    assert_eq!(unwrap(&[0, 0, 0, 17]), Some((17, b"".as_slice())));
    assert_eq!(unwrap(&wrap(17, &first)), Some((17, first.as_slice())));
    let host = HostKey::from_private(&encode_key(&[2; 32])).unwrap();
    let (_, mut c) = BrowserConnect::start(keys(1), &host.public).unwrap();
    *c.last_mut().unwrap() ^= 1;
    assert!(matches!(
        connect(keys(2), &c, |_| true),
        Err(Reject::UnknownDevice)
    ));
}

#[test]
fn record_credit_stalls_resumes_and_large_messages_do_not_deadlock() {
    let (bt, ht) = transports();
    let mut b = Records::new(bt);
    let mut h = Records::new(ht);
    let message = vec![42; crate::companion::MAX_MESSAGE_SIZE];
    b.queue(message.clone()).unwrap();
    assert!(b.queue(vec![0]).is_err());
    let mut frames = Vec::new();
    while let Some(frame) = b.next_frame().unwrap() {
        frames.push(frame);
    }
    assert!(b.sending());
    assert_eq!(b.send_credit(), 0);
    for frame in frames {
        assert!(h.receive(&frame).unwrap().is_none());
    }
    let c = h.next_frame().unwrap().unwrap();
    b.receive(&c).unwrap();
    let mut received = None;
    while let Some(f) = b.next_frame().unwrap() {
        if let Some(m) = h.receive(&f).unwrap() {
            received = Some(m);
        }
    }
    assert_eq!(received.unwrap(), message);
    assert!(!b.sending());
    h.handled();
    b.receive(&h.next_frame().unwrap().unwrap()).unwrap();
    assert_eq!(b.send_credit(), INITIAL_CREDIT);
    assert!(b
        .queue(vec![0; crate::companion::MAX_MESSAGE_SIZE + 1])
        .is_err());
}

#[test]
fn record_rejects_overflow_invalid_credit_ciphertext_and_order() {
    for record in [
        vec![],
        vec![9],
        vec![2, 0],
        vec![2, 0, 0, 0, 0],
        vec![2, 255, 255, 255, 255],
    ] {
        let (mut bt, ht) = transports();
        let mut h = Records::new(ht);
        let mut out = vec![0; record.len() + 16];
        bt.write_message(&record, &mut out).unwrap();
        assert!(h.receive(&frame(Kind::Data, &out)).is_err());
    }
    let (mut bt, ht) = transports();
    let mut h = Records::new(ht);
    for _ in 0..17 {
        let mut record = vec![0; MAX_RECORD_BODY + 1];
        record[0] = 0;
        let mut out = vec![0; record.len() + 16];
        bt.write_message(&record, &mut out).unwrap();
        let result = h.receive(&frame(Kind::Data, &out));
        if result.is_err() {
            return;
        }
        while h.next_frame().unwrap().is_some() {}
    }
    panic!("accepted an oversized message");
}

#[test]
fn secret_debug_is_redacted() {
    let (b, h, _, _) = pair("012345");
    let (bt, _) = transports();
    for debug in [
        format!("{b:?}"),
        format!("{h:?}"),
        format!("{:?}", Records::new(bt)),
        format!("{:?}", keys(1)),
    ] {
        assert!(debug.ends_with("([redacted])"));
        assert!(!debug.contains("012345"));
    }
}

#[test]
fn credit_is_owed_at_64_kib_and_transport_frames_are_authenticated() {
    let (mut browser, host) = transports();
    let mut host = Records::new(host);
    let mut record = vec![0; 32 * 1024 + 1];
    record[0] = 0;
    for step in 0..2 {
        let mut encrypted = vec![0; record.len() + 16];
        browser.write_message(&record, &mut encrypted).unwrap();
        assert!(host
            .receive(&frame(Kind::Data, &encrypted))
            .unwrap()
            .is_none());
        if step == 0 {
            assert!(host.next_frame().unwrap().is_none());
        }
    }
    let encrypted = host.next_frame().unwrap().unwrap();
    let mut credit = [0; 5];
    browser.read_message(&encrypted[1..], &mut credit).unwrap();
    assert_eq!(credit, [2, 0, 1, 0, 0]);
    let (bt, ht) = transports();
    let mut b = Records::new(bt);
    let mut h = Records::new(ht);
    b.queue(b"message".to_vec()).unwrap();
    let mut data = b.next_frame().unwrap().unwrap();
    *data.last_mut().unwrap() ^= 1;
    assert!(h.receive(&data).is_err());
    assert!(h.receive(&[2; 49]).is_err());

    let (mut browser, host) = transports();
    let mut host = Records::new(host);
    for record in [&[1][..], &[0, 42][..]] {
        let mut encrypted = vec![0; record.len() + 16];
        browser.write_message(record, &mut encrypted).unwrap();
        if host
            .receive(&frame(Kind::Data, &encrypted))
            .unwrap()
            .is_some()
        {
            host.handled();
        }
        assert!(host.next_frame().unwrap().is_none());
    }
}
