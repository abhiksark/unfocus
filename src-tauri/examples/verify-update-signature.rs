use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};

const MAX_KEY_BYTES: u64 = 8 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 8 * 1024;
const READ_BUFFER_BYTES: usize = 64 * 1024;

fn regular_file(path: &Path, label: &str, maximum_bytes: Option<u64>) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {label} {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "{label} must be a regular non-symlink file: {}",
            path.display()
        ));
    }
    if let Some(maximum) = maximum_bytes {
        if metadata.len() > maximum {
            return Err(format!(
                "{label} {} exceeds {maximum} bytes",
                path.display()
            ));
        }
    }
    Ok(())
}

fn read_outer_base64(path: &Path, label: &str, maximum_bytes: u64) -> Result<String, String> {
    regular_file(path, label, Some(maximum_bytes))?;
    let encoded = fs::read_to_string(path)
        .map_err(|error| format!("could not read {label} {}: {error}", path.display()))?;
    let encoded = encoded.strip_suffix('\n').unwrap_or(&encoded);
    if encoded.is_empty() || encoded.trim() != encoded || encoded.contains('\n') {
        return Err(format!(
            "{label} {} must contain one trimmed base64 value",
            path.display()
        ));
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| format!("{label} {} is not valid base64", path.display()))?;
    if STANDARD.encode(&decoded) != encoded {
        return Err(format!(
            "{label} {} is not canonical base64",
            path.display()
        ));
    }
    String::from_utf8(decoded)
        .map_err(|_| format!("{label} {} does not decode as UTF-8", path.display()))
}

fn expected_signature_path(data_path: &Path) -> PathBuf {
    let mut value: OsString = data_path.as_os_str().to_owned();
    value.push(".sig");
    PathBuf::from(value)
}

fn canonical_inner_packet(
    line: &str,
    label: &str,
    expected_bytes: usize,
) -> Result<Vec<u8>, String> {
    let decoded = STANDARD
        .decode(line)
        .map_err(|_| format!("{label} is not valid base64"))?;
    if decoded.len() != expected_bytes || STANDARD.encode(&decoded) != line {
        return Err(format!(
            "{label} is not canonical base64 of the expected size"
        ));
    }
    Ok(decoded)
}

fn strict_public_key_text(text: &str) -> Result<(), String> {
    if text.contains('\r') || !text.ends_with('\n') {
        return Err("updater public key must end with one LF and contain no CR".to_string());
    }
    let lines = text[..text.len() - 1].split('\n').collect::<Vec<_>>();
    if lines.len() != 2 {
        return Err("updater public key must contain exactly two minisign lines".to_string());
    }
    let displayed_id = lines[0]
        .strip_prefix("untrusted comment: minisign public key: ")
        .ok_or_else(|| "updater public key has an invalid untrusted comment".to_string())?;
    if displayed_id.len() != 16
        || !displayed_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    {
        return Err("updater public key comment has an invalid key ID".to_string());
    }
    let packet = canonical_inner_packet(lines[1], "updater public key packet", 42)?;
    if &packet[..2] != b"Ed" {
        return Err("updater public key packet has an unsupported algorithm".to_string());
    }
    let packet_id = packet[2..10]
        .iter()
        .rev()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    if packet_id != displayed_id {
        return Err("updater public key comment does not match its packet key ID".to_string());
    }
    Ok(())
}

fn strict_signature_text(text: &str) -> Result<(), String> {
    if text.contains('\r') || !text.ends_with('\n') {
        return Err("updater signature must end with one LF and contain no CR".to_string());
    }
    let lines = text[..text.len() - 1].split('\n').collect::<Vec<_>>();
    if lines.len() != 4 || lines[0] != "untrusted comment: signature from tauri secret key" {
        return Err("updater signature must contain exactly four Tauri minisign lines".to_string());
    }
    let packet = canonical_inner_packet(lines[1], "updater signature packet", 74)?;
    if &packet[..2] != b"ED" {
        return Err("updater signature must use the prehashed minisign algorithm".to_string());
    }
    canonical_inner_packet(lines[3], "updater signature global packet", 64)?;
    Ok(())
}

fn trusted_comment_names(signature: &Signature, filename: &str) -> bool {
    let Some(rest) = signature.trusted_comment().strip_prefix("timestamp:") else {
        return false;
    };
    let Some((timestamp, signed_filename)) = rest.split_once("\tfile:") else {
        return false;
    };
    !timestamp.is_empty()
        && timestamp.bytes().all(|byte| byte.is_ascii_digit())
        && signed_filename == filename
}

fn read_public_key(path: &Path) -> Result<PublicKey, String> {
    let public_key_text = read_outer_base64(path, "updater public key", MAX_KEY_BYTES)?;
    strict_public_key_text(&public_key_text)?;
    PublicKey::decode(&public_key_text)
        .map_err(|error| format!("could not decode updater public key: {error}"))
}

fn verify_update_signature(
    public_key_path: &Path,
    signature_path: &Path,
    data_path: &Path,
) -> Result<(), String> {
    regular_file(data_path, "signed input", None)?;
    if signature_path != expected_signature_path(data_path) {
        return Err(format!(
            "signature must be the adjacent file {}",
            expected_signature_path(data_path).display()
        ));
    }

    let public_key = read_public_key(public_key_path)?;
    let signature_text =
        read_outer_base64(signature_path, "updater signature", MAX_SIGNATURE_BYTES)?;
    strict_signature_text(&signature_text)?;
    let signature = Signature::decode(&signature_text)
        .map_err(|error| format!("could not decode updater signature: {error}"))?;
    let filename = data_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "signed input filename must be UTF-8".to_string())?;
    if !trusted_comment_names(&signature, filename) {
        return Err(format!(
            "updater signature trusted comment does not name {filename}"
        ));
    }

    let mut verifier = public_key.verify_stream(&signature).map_err(|error| {
        format!("could not initialize prehashed signature verification: {error}")
    })?;
    let mut file = File::open(data_path).map_err(|error| {
        format!(
            "could not open signed input {}: {error}",
            data_path.display()
        )
    })?;
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer).map_err(|error| {
            format!(
                "could not read signed input {}: {error}",
                data_path.display()
            )
        })?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count]);
    }
    verifier
        .finalize()
        .map_err(|error| format!("updater signature verification failed: {error}"))
}

fn usage() -> &'static str {
    "usage: cargo run --manifest-path src-tauri/Cargo.toml --example verify-update-signature -- --check-public-key <public-key> | <public-key> <signed-input.sig> <signed-input> [<signed-input.sig> <signed-input> ...]"
}

fn main() {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() == 2 && arguments[0] == "--check-public-key" {
        let public_key = PathBuf::from(&arguments[1]);
        if let Err(error) = read_public_key(&public_key) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        println!("validated updater public key {}", public_key.display());
        return;
    }
    if arguments.len() < 3 || !(arguments.len() - 1).is_multiple_of(2) {
        eprintln!("{}", usage());
        std::process::exit(2);
    }
    let public_key = PathBuf::from(&arguments[0]);
    let (pairs, remainder) = arguments[1..].as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for [signature_argument, data_argument] in pairs {
        let signature = PathBuf::from(signature_argument);
        let data = PathBuf::from(data_argument);
        if let Err(error) = verify_update_signature(&public_key, &signature, &data) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        println!("verified updater signature for {}", data.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::{
        io,
        sync::atomic::{AtomicU64, Ordering},
    };

    const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYyN0QyMUFCMUM3MTI1Q0YKUldUUEpYRWNxeUY5WXNpN3BwNzVoV0p1cklNczFsaFp1bzluK3hDa1FhR0JBZUtibkE0U3dFMjUK";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUUEpYRWNxeUY5WWphdEFpY1JJTm1UQm15NnJMR284SXFrS0JTa2p3UjdNVkRPZWpqM0x2NEp1cWsrdnpWeTBCOHBqeWsvcGNtMjNXclRuR3VuQUhiYUhCK1dGYmxzZGdFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzg4Mzg1Mjk2CWZpbGU6Zml4dHVyZS5BcHBJbWFnZQpsVEpYVHI2aG5yRnJMY1RPYWoySUdnNDkrNTdjT2xmbFBQLzlOQ3dzbDNjM1RQQjJ3bVV1ajI3U0djVlZ3bnFtOGdldkFyWVExK2lheUdwYkxTZ2FBUT09Cg==";
    const FIXTURE: &[u8] = b"fixture updater bytes\n";
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "unfocus-update-signature-test-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove test directory");
        }
    }

    fn fixture() -> (TestDirectory, PathBuf, PathBuf, PathBuf) {
        let directory = TestDirectory::new();
        let public_key = directory.path("test.key.pub");
        let data = directory.path("fixture.AppImage");
        let signature = expected_signature_path(&data);
        fs::write(&public_key, PUBLIC_KEY).expect("write public key");
        fs::write(&data, FIXTURE).expect("write fixture");
        fs::write(&signature, SIGNATURE).expect("write signature");
        (directory, public_key, signature, data)
    }

    #[test]
    fn verifies_tauri_prehashed_signature() {
        let (_directory, public_key, signature, data) = fixture();
        verify_update_signature(&public_key, &signature, &data).expect("valid signature");
    }

    #[test]
    fn validates_the_public_key_before_signing() {
        let (_directory, public_key, _signature, _data) = fixture();
        read_public_key(&public_key).expect("valid public key");
        fs::write(&public_key, "not a key\n").expect("replace public key");
        assert!(read_public_key(&public_key).is_err());
    }

    #[test]
    fn strict_inner_formats_reject_ignored_extra_lines() {
        let public_key = String::from_utf8(STANDARD.decode(PUBLIC_KEY).expect("decode public key"))
            .expect("public key UTF-8");
        let signature = String::from_utf8(STANDARD.decode(SIGNATURE).expect("decode signature"))
            .expect("signature UTF-8");
        strict_public_key_text(&public_key).expect("strict public key");
        strict_signature_text(&signature).expect("strict signature");
        assert!(strict_public_key_text(&format!("{public_key}extra\n")).is_err());
        assert!(strict_signature_text(&format!("{signature}extra\n")).is_err());
    }

    #[test]
    fn rejects_tampered_bytes() {
        let (_directory, public_key, signature, data) = fixture();
        fs::write(&data, b"tampered updater bytes\n").expect("tamper fixture");
        let error =
            verify_update_signature(&public_key, &signature, &data).expect_err("tamper must fail");
        assert!(error.contains("signature verification failed"), "{error}");
    }

    #[test]
    fn rejects_signature_not_adjacent_to_input() {
        let (directory, public_key, signature, data) = fixture();
        let moved = directory.path("moved.sig");
        fs::rename(signature, &moved).expect("move signature");
        let error = verify_update_signature(&public_key, &moved, &data)
            .expect_err("moved signature must fail");
        assert!(
            error.contains("signature must be the adjacent file"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_inputs() {
        let (directory, public_key, signature, data) = fixture();
        let target = directory.path("target.AppImage");
        fs::rename(&data, &target).expect("move data");
        symlink(&target, &data).expect("symlink data");
        let error =
            verify_update_signature(&public_key, &signature, &data).expect_err("symlink must fail");
        assert!(error.contains("regular non-symlink file"), "{error}");
    }

    #[test]
    fn rejects_noncanonical_outer_base64() {
        let (_directory, public_key, signature, data) = fixture();
        fs::write(&signature, format!(" {}", SIGNATURE)).expect("rewrite signature");
        let error = verify_update_signature(&public_key, &signature, &data)
            .expect_err("whitespace must fail");
        assert!(error.contains("one trimmed base64 value"), "{error}");
    }

    #[test]
    fn trusted_comment_must_name_input() {
        let (directory, public_key, signature, data) = fixture();
        let renamed = directory.path("renamed.AppImage");
        fs::rename(&data, &renamed).expect("rename data");
        let renamed_signature = expected_signature_path(&renamed);
        fs::rename(signature, &renamed_signature).expect("rename signature");
        let error = verify_update_signature(&public_key, &renamed_signature, &renamed)
            .expect_err("trusted filename mismatch must fail");
        assert!(error.contains("trusted comment does not name"), "{error}");
    }

    #[test]
    fn io_errors_keep_paths_but_never_signature_contents() {
        let directory = TestDirectory::new();
        let public_key = directory.path("missing.pub");
        let data = directory.path("missing.AppImage");
        let signature = expected_signature_path(&data);
        let error = verify_update_signature(&public_key, &signature, &data)
            .expect_err("missing data must fail");
        assert!(error.contains("missing.AppImage"), "{error}");
        assert!(!error.contains(SIGNATURE));
        assert!(!error.contains(PUBLIC_KEY));
    }

    #[test]
    fn expected_signature_path_appends_instead_of_replacing_extension() {
        assert_eq!(
            expected_signature_path(Path::new("Unfocus.AppImage")),
            PathBuf::from("Unfocus.AppImage.sig")
        );
        assert_eq!(
            expected_signature_path(Path::new("payload.json")),
            PathBuf::from("payload.json.sig")
        );
    }

    #[test]
    fn read_loop_uses_io_eof_without_trusting_metadata_size() {
        let mut cursor = io::Cursor::new(FIXTURE);
        let mut bytes = Vec::new();
        cursor.read_to_end(&mut bytes).expect("read fixture");
        assert_eq!(bytes, FIXTURE);
    }
}
