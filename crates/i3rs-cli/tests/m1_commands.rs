#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use i3rs_core::m1::log_image::{
    LOG_IMAGE_RECORD_TYPE, PROPERTIES_RECORD_TYPE, PROPERTY_RECORD_TYPE, STRING_RECORD_TYPE,
};
use i3rs_core::m1::log_info::{
    ACTIVE_USER_RECORD_TYPE, DEVICE_INFO_RECORD_TYPE, INFO_RECORD_TYPE, LOG_SYSTEM_RECORD_TYPE,
    PERMISSIONS_RECORD_TYPE,
};
use i3rs_core::m1::xcp::{decode_datagram, encode_frame};
use serde_json::Value;

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_i3rs"))
        .args(args)
        .output()
        .unwrap()
}
struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "i3rs-m1-cli-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("image.mo");
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn record(kind: u32, control: u8, encoding: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = b"Mo".to_vec();
    bytes.extend_from_slice(&[control, encoding]);
    bytes.extend_from_slice(&kind.to_be_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
fn joined(records: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        bytes.extend(record);
    }
    bytes
}

fn property(name: &[u8], value_encoding: u8, value: &[u8]) -> Vec<u8> {
    record(
        PROPERTY_RECORD_TYPE,
        0,
        0,
        &joined(&[
            record(STRING_RECORD_TYPE, 0, 0, name),
            record(STRING_RECORD_TYPE, 1, value_encoding, value),
        ]),
    )
}
fn success_json(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn m1_dispatch_and_nested_help_are_recognized_without_legacy_file_fallback() {
    let output = cli(&["m1", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for command in [
        "devices",
        "inspect",
        "download-raw",
        "download",
        "convert-image",
        "convert-capture",
        "image-info",
    ] {
        assert!(help.contains(command));
    }
    assert!(help.contains("bundle"));
    assert!(help.contains("unvalidated"));
    assert!(help.contains("official M1 image"));
    let output = cli(&["m1", "download-raw", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("1 through 8"));
    assert!(help.contains("--destination"));
    assert!(help.contains("--max-total-bytes"));
    assert!(help.contains("--reconnect-attempts"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("could not open"));
}

#[test]
fn invalid_systems_and_endpoints_are_rejected_before_network_io() {
    let base = TestDirectory::new();
    for systems in ["0", "9", "1,1"] {
        let output = cli(&[
            "m1",
            "download-raw",
            "127.0.0.1:9",
            "--systems",
            systems,
            "--destination",
            base.0.to_str().unwrap(),
        ]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("1 through 8") || error.contains("distinct"),
            "{error}"
        );
        assert!(!error.contains("timed out"));
    }
    for attempts in ["9", "255", "-1"] {
        let output = cli(&[
            "m1",
            "download-raw",
            "127.0.0.1:9",
            "--systems",
            "1",
            "--destination",
            base.0.to_str().unwrap(),
            "--reconnect-attempts",
            attempts,
        ]);
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("timed out"));
    }
    for endpoint in [
        "0.0.0.0:5555",
        "224.0.0.1:5555",
        "255.255.255.255:5555",
        "127.0.0.1:0",
        "[::]:5555",
        "[ff02::1%3]:5555",
        "[fe80::1]:5555",
        "[::ffff:224.0.0.1]:5555",
    ] {
        let output = cli(&["m1", "inspect", endpoint, "--timeout-ms", "1"]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("endpoint"), "{endpoint}: {error}");
        assert!(!error.contains("timed out"));
    }
    assert_eq!(fs::read_dir(&base.0).unwrap().count(), 0);
}

#[test]
fn discovery_windows_are_bounded_without_starting_a_browse() {
    for window in ["0", "60001"] {
        let output = cli(&["m1", "devices", "--window-ms", window]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("window-ms"));
    }
}

#[test]
fn image_info_preserves_non_utf8_names_and_opaque_value_bytes_as_bounded_hex() {
    let directory = TestDirectory::new();
    let properties = joined(&[
        property(&[0xff, 0, b'A'], 10, &[0xde, 0xad, 0]),
        property(&[b'x'; 300], 0, b"value"),
    ]);
    let root = record(
        LOG_IMAGE_RECORD_TYPE,
        0,
        0,
        &record(PROPERTIES_RECORD_TYPE, 0, 0, &properties),
    );
    let path = directory.file(&root);
    let json = success_json(cli(&["m1", "image-info", path.to_str().unwrap()]));
    assert_eq!(json["converter_validated"], false);
    assert_eq!(json["checksums_validated"], false);
    assert_eq!(json["produces_ld"], false);
    let property = &json["groups"][0]["properties"][0];
    assert_eq!(property["name"]["hex"], "ff0041");
    assert!(property["name"]["utf8"].is_null());
    assert_eq!(property["value"]["opaque"], true);
    assert_eq!(property["value"]["payload_preview"]["hex"], "dead00");
    let long = &json["groups"][0]["properties"][1]["name"];
    assert_eq!(long["total_bytes"], 300);
    assert_eq!(long["truncated"], true);
    assert_eq!(long["hex"].as_str().unwrap().len(), 512);
}

#[test]
fn image_info_limits_rendered_properties_and_exposes_transformed_root() {
    let directory = TestDirectory::new();
    let mut properties = Vec::new();
    for _ in 0..257 {
        properties.resize(properties.len().next_multiple_of(4), 0);
        properties.extend_from_slice(&property(b"name", 0, b"value"));
    }
    let bytes = record(
        LOG_IMAGE_RECORD_TYPE,
        0,
        0,
        &record(PROPERTIES_RECORD_TYPE, 0, 0, &properties),
    );
    let path = directory.file(&bytes);
    let json = success_json(cli(&["m1", "image-info", path.to_str().unwrap()]));
    assert_eq!(json["rendered_properties"], 256);
    assert_eq!(json["groups"][0]["properties_omitted"], 1);
    assert_eq!(
        json["groups"][0]["properties"].as_array().unwrap().len(),
        256
    );

    let path = directory.file(&record(LOG_IMAGE_RECORD_TYPE, 5, 30, &[0xff, 0, 0xab]));
    let json = success_json(cli(&["m1", "image-info", path.to_str().unwrap()]));
    assert_eq!(json["opaque_root"], true);
    assert_eq!(json["header"]["control_byte_2"], 5);
    assert_eq!(json["opaque_payload_preview"]["hex"], "ff00ab");
    assert!(json["groups"].as_array().unwrap().is_empty());
}

#[test]
fn image_info_rejects_malformed_declared_lengths_and_file_size_caps() {
    let directory = TestDirectory::new();
    let path = directory.file(&[0; 20]);
    let output = cli(&[
        "m1",
        "image-info",
        path.to_str().unwrap(),
        "--max-bytes",
        "12",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeding --max-bytes"));
    let output = cli(&["m1", "image-info", path.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("signature"));
    let mut bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &[]);
    bytes[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    let path = directory.file(&bytes);
    let output = cli(&["m1", "image-info", path.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("truncated"));
}

#[test]
fn loopback_inspect_outputs_wire_ids_and_human_system_numbers() {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let endpoint = socket.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut device = [0; 56];
        device[16..20].copy_from_slice(&1_u32.to_be_bytes());
        device[40..44].copy_from_slice(&9_u32.to_be_bytes());
        device[44..48].copy_from_slice(&1_u32.to_be_bytes());
        let mut system = [0; 16];
        system[8..12].copy_from_slice(&8_u32.to_be_bytes());
        system[12..16].copy_from_slice(&10_u32.to_be_bytes());
        let info = record(
            INFO_RECORD_TYPE,
            0,
            0,
            &[
                record(DEVICE_INFO_RECORD_TYPE, 0, 0, &device),
                record(LOG_SYSTEM_RECORD_TYPE, 1, 0, &system),
            ]
            .concat(),
        );
        let mut permissions = [0; 72];
        permissions[65] = 1;
        let rights = record(
            INFO_RECORD_TYPE,
            0,
            0,
            &record(
                ACTIVE_USER_RECORD_TYPE,
                0,
                0,
                &record(PERMISSIONS_RECORD_TYPE, 0, 0, &permissions),
            ),
        );
        let mut selected = Vec::new();
        let mut offset = 0;
        let mut input = [0; 1024];
        loop {
            let (len, peer) = socket.recv_from(&mut input).unwrap();
            let frames = decode_datagram(&input[..len]).unwrap();
            assert_eq!(frames.len(), 1);
            let frame = frames[0];
            let response = match frame.payload[0] {
                0xff => vec![0xff, 0, 0, 64, 64, 0],
                0xf6 => {
                    assert_eq!(frame.payload[3], 0x26);
                    let selector = u32::from_le_bytes(frame.payload[4..8].try_into().unwrap());
                    selected = match selector {
                        0x10062 => info.clone(),
                        0x10040 => rights.clone(),
                        other => panic!("unexpected selector {other:x}"),
                    };
                    offset = 0;
                    vec![0xff]
                }
                0xf5 => {
                    let end = offset + usize::from(frame.payload[1]);
                    let mut bytes = vec![0xff];
                    bytes.extend_from_slice(&selected[offset..end]);
                    offset = end;
                    bytes
                }
                0xfe => {
                    socket
                        .send_to(&encode_frame(frame.counter, &[0xff]).unwrap(), peer)
                        .unwrap();
                    break;
                }
                other => panic!("unexpected command {other:x}"),
            };
            socket
                .send_to(&encode_frame(frame.counter, &response).unwrap(), peer)
                .unwrap();
        }
    });
    let json = success_json(cli(&[
        "m1",
        "inspect",
        &endpoint.to_string(),
        "--timeout-ms",
        "1000",
    ]));
    server.join().unwrap();
    assert_eq!(json["endpoint"], endpoint.to_string());
    assert_eq!(json["permissions_available"], true);
    assert_eq!(json["systems"][0]["id"], 1);
    assert_eq!(json["systems"][0]["system_number"], 2);
    assert_eq!(json["systems"][0]["sector_count"], 2);
    assert_eq!(json["download_allowed"][1], true);
    assert_eq!(json["download_allowed"][0], false);
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
        }
    }
    !crc
}
fn varint(mut value: usize, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn synthetic_xz(data: &[u8]) -> Vec<u8> {
    let flags = [0, 1];
    let mut x = b"\xfd7zXZ\0".to_vec();
    x.extend(flags);
    x.extend(crc32(&flags).to_le_bytes());
    let header = [2, 0, 0x21, 1, 0, 0, 0, 0];
    x.extend(header);
    x.extend(crc32(&header).to_le_bytes());
    x.push(1);
    x.extend(((data.len() - 1) as u16).to_be_bytes());
    x.extend(data);
    x.push(0);
    x.resize(x.len().next_multiple_of(4), 0);
    x.extend(crc32(data).to_le_bytes());
    let mut index = vec![0, 1];
    varint(12 + data.len() + 8, &mut index);
    varint(data.len(), &mut index);
    index.resize(index.len().next_multiple_of(4), 0);
    index.extend(crc32(&index).to_le_bytes());
    let back = (index.len() / 4 - 1) as u32;
    x.extend(index);
    let mut footer = back.to_le_bytes().to_vec();
    footer.extend(flags);
    x.extend(crc32(&footer).to_le_bytes());
    x.extend(footer);
    x.extend(b"YZ");
    x
}
fn native_parts() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut setup_info = vec![0; 16];
    setup_info.extend([1; 16]);
    let group = [1u32, 7816, 0, 1, 4]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect::<Vec<_>>();
    let entry = [1u32, 0x100, 1]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect::<Vec<_>>();
    let setups = record(
        0x0200_0020,
        0,
        0,
        &record(
            0x0200_0022,
            0,
            0,
            &joined(&[
                record(0x0201_0023, 0, 0, &setup_info),
                record(0x0201_0044, 0, 0, &[2; 16]),
                record(
                    0x0200_0024,
                    0,
                    0,
                    &joined(&[
                        record(0x0201_0025, 0, 0, &group),
                        record(0x0201_0026, 0, 0, &entry),
                    ]),
                ),
            ]),
        ),
    );
    let xml=br#"<TuneConfig><System><TargetModel><Events><Event Index="0" Period="1"/></Events></TargetModel></System><Device><Items><BuiltIn.Channel Name="Speed"><Properties><Value Type="f32" Unit="rpm"><Address Base="1" Offset="256"/></Value></Properties></BuiltIn.Channel></Items></Device></TuneConfig>"#;
    let inner = record(0x0201_002c, 0, 0, xml);
    let mut compressed = (inner.len() as u32).to_be_bytes().to_vec();
    compressed.extend(synthetic_xz(&inner));
    let metadata = record(
        0x0200_0046,
        0,
        0,
        &record(
            0x0200_0047,
            0,
            0,
            &joined(&[
                record(0x0201_0044, 0, 0, &[2; 16]),
                record(0x0201_002c, 0, 31, &compressed),
            ]),
        ),
    );
    let mut packets = vec![
        record(0x0201_004d, 0, 0, &0u32.to_be_bytes()),
        record(0x0201_0039, 0, 0, &[1; 16]),
        record(0x0201_0029, 0, 0, &0u32.to_be_bytes()),
    ];
    for (ticks, value) in [(0u32, 1000.0f32), (125_000_000, 2000.0)] {
        let mut p = ticks.to_be_bytes().to_vec();
        p.extend(value.to_bits().to_be_bytes());
        p.extend(crc32(&p).to_be_bytes());
        packets.push(record(0x0201_002b, 0, 0, &p));
    }
    let sector = record(0x0200_004c, 0, 0, &joined(&packets));
    (setups, metadata, sector)
}
fn synthetic_native_image() -> Vec<u8> {
    synthetic_native_image_with_sectors(Vec::new())
}

fn synthetic_native_image_with_sectors(mut extra_sectors: Vec<Vec<u8>>) -> Vec<u8> {
    let (s, m, d) = native_parts();
    extra_sectors.insert(0, d);
    let version = record(
        PROPERTY_RECORD_TYPE,
        0,
        0,
        &joined(&[
            record(STRING_RECORD_TYPE, 0, 0, b"version"),
            record(0x0101_0008, 0, 0, &[2, 3, 0, 0, 0]),
        ]),
    );
    record(
        LOG_IMAGE_RECORD_TYPE,
        0,
        0,
        &joined(&[
            record(PROPERTIES_RECORD_TYPE, 0, 0, &version),
            s,
            m,
            record(0x0101_0002, 0, 0, &joined(&extra_sectors)),
        ]),
    )
}
fn assert_converted_json(json: &Value, state: &str) {
    assert_eq!(json["manifest"]["state"], state);
    assert_eq!(json["manifest"]["vendor_compatibility_validated"], false);
    assert!(!json["manifest"]["warnings"].as_array().unwrap().is_empty());
    let directory = PathBuf::from(json["directory"].as_str().unwrap());
    let files = json["manifest"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    let path = directory.join(files[0]["file_name"].as_str().unwrap());
    let ld = i3rs_core::LdFile::open(&path).unwrap();
    assert_eq!(ld.channels.len(), 1);
    assert_eq!(ld.channels[0].name, "Speed");
    assert_eq!(
        ld.read_channel_data(&ld.channels[0]).unwrap(),
        vec![1000.0, 2000.0]
    );
}
#[test]
fn convert_image_cli_writes_native_ld_and_returns_manifest_warnings() {
    let directory = TestDirectory::new();
    let path = directory.file(&synthetic_native_image());
    let json = success_json(cli(&[
        "m1",
        "convert-image",
        path.to_str().unwrap(),
        "--destination",
        directory.0.to_str().unwrap(),
    ]));
    assert_eq!(json["schema_version"], "i3rs-cli.m1.conversion.v1");
    assert_converted_json(&json, "complete");
    assert_eq!(fs::read(path).unwrap(), synthetic_native_image());
}

#[test]
fn convert_image_cli_reports_partial_success_when_a_system_cannot_be_decoded() {
    let directory = TestDirectory::new();
    let mut packet = 0u32.to_be_bytes().to_vec();
    packet.extend(1000f32.to_bits().to_be_bytes());
    packet.extend(crc32(&packet).to_be_bytes());
    let unsynchronized_sector = record(
        0x0200_004c,
        1,
        0,
        &joined(&[
            record(0x0201_004d, 0, 0, &0u32.to_be_bytes()),
            record(0x0201_002b, 0, 0, &packet),
        ]),
    );
    let path = directory.file(&synthetic_native_image_with_sectors(vec![
        unsynchronized_sector,
    ]));
    let output = cli(&[
        "m1",
        "convert-image",
        path.to_str().unwrap(),
        "--destination",
        directory.0.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("omitted 1 logging system"));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_converted_json(&json, "partial");
    assert_eq!(json["manifest"]["omitted_systems"], serde_json::json!([1]));
    assert_eq!(json["manifest"]["files"][0]["system"], 0);
    let saved: Value = serde_json::from_slice(
        &fs::read(PathBuf::from(json["directory"].as_str().unwrap()).join("manifest.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved, json["manifest"]);
}

#[test]
fn conversion_errors_are_bounded_and_do_not_create_outputs() {
    let directory = TestDirectory::new();
    let path = directory.file(&[0; 20]);
    for args in [
        vec![
            "m1",
            "convert-image",
            path.to_str().unwrap(),
            "--destination",
            directory.0.to_str().unwrap(),
        ],
        vec![
            "m1",
            "convert-image",
            path.to_str().unwrap(),
            "--destination",
            directory.0.to_str().unwrap(),
            "--max-input-bytes",
            "12",
        ],
        vec![
            "m1",
            "convert-capture",
            path.to_str().unwrap(),
            "--destination",
            directory.0.to_str().unwrap(),
        ],
        vec![
            "m1",
            "convert-image",
            directory
                .0
                .join("missing-i3rs-test-image")
                .to_str()
                .unwrap(),
            "--destination",
            directory.0.to_str().unwrap(),
        ],
        vec![
            "m1",
            "convert-image",
            path.to_str().unwrap(),
            "--destination",
            directory.0.to_str().unwrap(),
            "--max-records",
            "0",
        ],
    ] {
        let output = cli(&args);
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    let output = cli(&[
        "m1",
        "download",
        "127.0.0.1:9",
        "--systems",
        "1,1",
        "--destination",
        directory.0.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("distinct"));
    for command in ["download", "convert-image", "convert-capture"] {
        let output = cli(&["m1", command, "--help"]);
        assert!(output.status.success());
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.contains("--max-input-bytes"));
        assert!(help.contains("--max-output-file-bytes"));
    }
}
fn native_download_device() -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let (setups, metadata, sector) = native_parts();
    let mut system = [0; 16];
    system[12..16].copy_from_slice(&1u32.to_be_bytes());
    let mut permissions = [0; 72];
    permissions[64] = 1;
    let info = record(
        INFO_RECORD_TYPE,
        0,
        0,
        &joined(&[
            record(LOG_SYSTEM_RECORD_TYPE, 0, 0, &system),
            record(
                ACTIVE_USER_RECORD_TYPE,
                0,
                0,
                &record(PERMISSIONS_RECORD_TYPE, 0, 0, &permissions),
            ),
        ]),
    );
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let endpoint = socket.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut selected = Vec::new();
        let mut position = 0;
        let mut input = [0; 1024];
        loop {
            let (n, peer) = socket.recv_from(&mut input).unwrap();
            let frames = decode_datagram(&input[..n]).unwrap();
            assert_eq!(frames.len(), 1);
            let frame = frames[0];
            let reply = match frame.payload[0] {
                0xff => vec![0xff, 0, 0, 64, 64, 0],
                0xf6 => {
                    let ext = frame.payload[3];
                    let addr = u32::from_le_bytes(frame.payload[4..8].try_into().unwrap());
                    selected = match (ext, addr) {
                        (0x26, 0x10062) => info.clone(),
                        (0x27, 0) => setups.clone(),
                        (0x2a, 0) => metadata.clone(),
                        (0x25, 0) => sector.clone(),
                        other => panic!("unexpected read selector {other:?}"),
                    };
                    position = 0;
                    vec![0xff]
                }
                0xf5 => {
                    let end = position + usize::from(frame.payload[1]);
                    let mut reply = vec![0xff];
                    reply.extend(&selected[position..end]);
                    position = end;
                    reply
                }
                0xfe => {
                    socket
                        .send_to(&encode_frame(frame.counter, &[0xff]).unwrap(), peer)
                        .unwrap();
                    break;
                }
                other => panic!("non-read-only command {other:x}"),
            };
            socket
                .send_to(&encode_frame(frame.counter, &reply).unwrap(), peer)
                .unwrap();
        }
    });
    (endpoint, server)
}

#[test]
fn loopback_download_captures_and_converts_with_read_only_commands() {
    let directory = TestDirectory::new();
    let (endpoint, server) = native_download_device();
    let output = cli(&[
        "m1",
        "download",
        &endpoint.to_string(),
        "--systems",
        "1",
        "--destination",
        directory.0.to_str().unwrap(),
        "--timeout-ms",
        "1000",
    ]);
    let json = success_json(output);
    server.join().unwrap();
    assert_eq!(json["schema_version"], "i3rs-cli.m1.download.v1");
    assert_eq!(json["raw_manifest"]["state"], "complete");
    assert!(
        PathBuf::from(json["raw_directory"].as_str().unwrap())
            .join("manifest.json")
            .is_file()
    );
    assert_converted_json(&json, "complete");
    let json = success_json(cli(&[
        "m1",
        "convert-capture",
        json["raw_directory"].as_str().unwrap(),
        "--destination",
        directory.0.to_str().unwrap(),
    ]));
    assert_converted_json(&json, "complete");

    // Also verify --recover-partial on an interrupted bundle.
    let partial_dir = TestDirectory::new();
    let raw_dir = PathBuf::from(json["manifest"]["source"]["path"].as_str().unwrap());
    let mut partial_manifest: Value =
        serde_json::from_slice(&fs::read(raw_dir.join("manifest.json")).unwrap()).unwrap();
    let after = partial_manifest["files"]
        .as_array_mut()
        .unwrap()
        .pop()
        .unwrap();
    let after_bytes = after["bytes"].as_u64().unwrap();
    partial_manifest["total_bytes"] =
        Value::from(partial_manifest["total_bytes"].as_u64().unwrap() - after_bytes);
    partial_manifest["state"] = Value::from("incomplete");
    partial_manifest["boundaries_unchanged"] = Value::Null;
    for entry in partial_manifest["files"].as_array().unwrap() {
        let name = entry["file_name"].as_str().unwrap();
        fs::copy(raw_dir.join(name), partial_dir.0.join(name)).unwrap();
    }
    fs::write(
        partial_dir.0.join("manifest.json"),
        serde_json::to_vec_pretty(&partial_manifest).unwrap(),
    )
    .unwrap();
    let recovered = success_json(cli(&[
        "m1",
        "convert-capture",
        partial_dir.0.to_str().unwrap(),
        "--recover-partial",
        "--destination",
        directory.0.to_str().unwrap(),
    ]));
    assert_converted_json(&recovered, "partial");
    assert!(
        recovered["manifest"]["files"][0]["file_name"]
            .as_str()
            .unwrap()
            .starts_with("partial-")
    );
}

#[test]
fn download_conversion_failure_reports_and_preserves_complete_raw_bundle() {
    let directory = TestDirectory::new();
    let (endpoint, server) = native_download_device();
    let output = cli(&[
        "m1",
        "download",
        &endpoint.to_string(),
        "--systems",
        "1",
        "--destination",
        directory.0.to_str().unwrap(),
        "--timeout-ms",
        "1000",
        "--max-samples",
        "1",
    ]);
    assert!(!output.status.success());
    server.join().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("raw capture retained at"), "{error}");
    assert!(error.contains("sample limit"), "{error}");
    let children = fs::read_dir(&directory.0)
        .unwrap()
        .map(|r| r.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    let manifest: Value =
        serde_json::from_slice(&fs::read(children[0].join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["state"], "complete");
    assert!(children[0].join("system-00-sector-0000000000.mo").is_file());
}
