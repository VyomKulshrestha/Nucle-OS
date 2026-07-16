//! # Passphrase-protected envelope encryption for pool state at rest
//!
//! Encrypts `pool_dir/state.json` -- the file holding the actual pool
//! (every stored file's DNA strands) and catalog -- so it's no longer
//! recoverable in the clear by anyone who can read the pool directory.
//! Deliberately scoped to just that file: `audit.log` and `config.json`
//! remain plaintext (filenames/timestamps, not file content), a real and
//! documented limitation rather than a silent gap.
//!
//! **Envelope encryption, not direct passphrase encryption.** A random
//! 256-bit data-encryption key (DEK) actually encrypts `state.json`
//! (via ChaCha20-Poly1305, a fresh random nonce per write); the DEK
//! itself is wrapped by a key derived from your passphrase (via
//! Argon2id, a deliberately slow, memory-hard KDF) and stored in
//! `pool_dir/key.json`. This split matters for more than architectural
//! taste: the slow KDF only has to run once, when a pool is unlocked
//! (`NucleOS::open_encrypted`), not on every single `persist()` call a
//! `store`/`retrieve`/`migrate` triggers -- direct passphrase encryption
//! would otherwise add real, noticeable per-command latency.
//!
//! **Honest about what this does and doesn't protect against.** A wrong
//! passphrase and a corrupted key file deliberately produce the same
//! generic error (`unlock_key_file`) -- distinguishing them would leak
//! information a real system shouldn't. Forgetting the passphrase means
//! permanent data loss: there is no recovery mechanism, by design, since
//! adding one would be a backdoor. This protects data *at rest* (a
//! stolen disk, a copied `pool_dir`) -- it does not protect data while a
//! pool is open in memory during a command, and it does not protect
//! `audit.log`/`config.json`.

use chacha20poly1305::aead::{Aead, Generate, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use std::path::Path;

const KEY_FILE_NAME: &str = "key.json";
pub const DEK_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// Raw, in-memory data-encryption key for one unlocked pool.
pub type Dek = [u8; DEK_LEN];

#[derive(Debug, Serialize, Deserialize)]
struct KeyFile {
    /// Random salt fed into Argon2id alongside the passphrase.
    salt: Vec<u8>,
    /// Nonce used for the one AEAD call that wraps the DEK below.
    wrap_nonce: Vec<u8>,
    /// The DEK, encrypted under the passphrase-derived wrapping key.
    wrapped_dek: Vec<u8>,
}

/// True if `pool_dir` has been set up for encryption (`key.json` exists).
/// A pool with no key file is a plain, unencrypted pool -- today's
/// unchanged default behavior.
pub fn is_encrypted(pool_dir: &Path) -> bool {
    pool_dir.join(KEY_FILE_NAME).exists()
}

fn derive_wrapping_key(passphrase: &str, salt: &[u8]) -> Result<Key, String> {
    let mut key_bytes = [0u8; DEK_LEN];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key_bytes)
        .map_err(|e| format!("passphrase key derivation failed: {}", e))?;
    Ok(Key::from(key_bytes))
}

/// Generates a new random DEK, wraps it with a key derived from
/// `passphrase` (a fresh random salt each call, so encrypting the same
/// passphrase twice never produces the same wrapping key), and writes
/// `pool_dir/key.json` (atomic temp-file-then-rename, same convention as
/// `state.json`/`config.json`). Returns the raw DEK so the caller can
/// immediately re-encrypt an existing `state.json` with it.
pub fn create_key_file(pool_dir: &Path, passphrase: &str) -> Result<Dek, String> {
    std::fs::create_dir_all(pool_dir)
        .map_err(|e| format!("failed to create pool directory '{}': {}", pool_dir.display(), e))?;

    let dek: Dek = Generate::generate();
    let salt: [u8; SALT_LEN] = Generate::generate();

    let wrapping_key = derive_wrapping_key(passphrase, &salt)?;
    let cipher = ChaCha20Poly1305::new(&wrapping_key);
    let wrap_nonce = Nonce::generate();
    let wrapped_dek = cipher
        .encrypt(&wrap_nonce, dek.as_ref())
        .map_err(|e| format!("failed to wrap data key: {}", e))?;

    let key_file = KeyFile {
        salt: salt.to_vec(),
        wrap_nonce: wrap_nonce.to_vec(),
        wrapped_dek,
    };
    write_key_file(pool_dir, &key_file)?;
    Ok(dek)
}

fn write_key_file(pool_dir: &Path, key_file: &KeyFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(key_file)
        .map_err(|e| format!("failed to serialize key file: {}", e))?;
    let tmp_path = pool_dir.join(format!("{}.tmp", KEY_FILE_NAME));
    std::fs::write(&tmp_path, &json)
        .map_err(|e| format!("failed to write key file to '{}': {}", tmp_path.display(), e))?;
    std::fs::rename(&tmp_path, pool_dir.join(KEY_FILE_NAME))
        .map_err(|e| format!("failed to finalize key file: {}", e))
}

/// Reads `pool_dir/key.json` and unwraps the DEK using `passphrase`. A
/// wrong passphrase and a corrupted/tampered key file both surface as
/// this same generic error (an AEAD authentication failure) -- see this
/// module's own doc comment for why that's deliberate, not an omission.
pub fn unlock_key_file(pool_dir: &Path, passphrase: &str) -> Result<Dek, String> {
    let path = pool_dir.join(KEY_FILE_NAME);
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read key file at '{}': {}", path.display(), e))?;
    let key_file: KeyFile = serde_json::from_str(&json)
        .map_err(|e| format!("failed to parse key file at '{}': {}", path.display(), e))?;

    let wrapping_key = derive_wrapping_key(passphrase, &key_file.salt)?;
    let cipher = ChaCha20Poly1305::new(&wrapping_key);
    let nonce = Nonce::try_from(key_file.wrap_nonce.as_slice())
        .map_err(|_| "failed to unlock pool: key file has a malformed nonce".to_string())?;
    let dek_vec = cipher
        .decrypt(&nonce, key_file.wrapped_dek.as_ref())
        .map_err(|_| "failed to unlock pool: wrong passphrase, or a corrupted key file".to_string())?;

    Dek::try_from(dek_vec.as_slice())
        .map_err(|_| "failed to unlock pool: key file has an unexpected key length".to_string())
}

/// Removes `pool_dir/key.json`, turning off encryption for this pool.
/// A missing key file is not an error -- encryption is already off.
pub fn remove_key_file(pool_dir: &Path) -> Result<(), String> {
    let path = pool_dir.join(KEY_FILE_NAME);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("failed to remove key file at '{}': {}", path.display(), e))?;
    }
    Ok(())
}

/// Encrypts `plaintext` under `dek` with a fresh random 96-bit nonce
/// (prepended to the returned ciphertext) -- safe to call repeatedly
/// with the same key, since nonce reuse across any realistic number of
/// `persist()` calls is astronomically unlikely.
pub fn encrypt_bytes(dek: &Dek, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new(&Key::from(*dek));
    let nonce = Nonce::generate();
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("failed to encrypt pool state: {}", e))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Reverses [`encrypt_bytes`]: splits the leading nonce back off and
/// decrypts the remainder under `dek`.
pub fn decrypt_bytes(dek: &Dek, data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < NONCE_LEN {
        return Err("encrypted pool state is truncated (shorter than one nonce)".to_string());
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let cipher = ChaCha20Poly1305::new(&Key::from(*dek));
    let nonce = Nonce::try_from(nonce_bytes)
        .map_err(|_| "encrypted pool state has a malformed nonce".to_string())?;
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| "failed to decrypt pool state: wrong key, or corrupted/tampered data".to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nucle_vfs_crypto_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn a_fresh_pool_dir_is_not_encrypted() {
        let dir = scratch_dir("fresh");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!is_encrypted(&dir));
    }

    #[test]
    fn create_then_unlock_with_the_right_passphrase_recovers_the_same_dek() {
        let dir = scratch_dir("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        let dek = create_key_file(&dir, "correct horse battery staple").unwrap();
        assert!(is_encrypted(&dir));

        let unlocked = unlock_key_file(&dir, "correct horse battery staple").unwrap();
        assert_eq!(dek, unlocked);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unlocking_with_the_wrong_passphrase_fails_clearly() {
        let dir = scratch_dir("wrong_passphrase");
        let _ = std::fs::remove_dir_all(&dir);

        create_key_file(&dir, "correct horse battery staple").unwrap();
        let result = unlock_key_file(&dir, "not the right passphrase");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("wrong passphrase"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_key_files_for_the_same_passphrase_are_not_identical() {
        // Different random salt/nonce each call -- proves the salt is
        // actually random, not a fixed/reused value.
        let dir_a = scratch_dir("salt_a");
        let dir_b = scratch_dir("salt_b");
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);

        create_key_file(&dir_a, "same passphrase").unwrap();
        create_key_file(&dir_b, "same passphrase").unwrap();

        let a = std::fs::read_to_string(dir_a.join(KEY_FILE_NAME)).unwrap();
        let b = std::fs::read_to_string(dir_b.join(KEY_FILE_NAME)).unwrap();
        assert_ne!(a, b);

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[test]
    fn remove_key_file_turns_off_encryption() {
        let dir = scratch_dir("remove");
        let _ = std::fs::remove_dir_all(&dir);

        create_key_file(&dir, "a passphrase").unwrap();
        assert!(is_encrypted(&dir));

        remove_key_file(&dir).unwrap();
        assert!(!is_encrypted(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_a_key_file_that_does_not_exist_is_not_an_error() {
        let dir = scratch_dir("remove_missing");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(remove_key_file(&dir).is_ok());
    }

    #[test]
    fn encrypt_then_decrypt_bytes_roundtrips() {
        let dek: Dek = Generate::generate();
        let plaintext = b"a whole pool's worth of DNA strand data, imagine it's bigger";

        let ciphertext = encrypt_bytes(&dek, plaintext).unwrap();
        assert_ne!(ciphertext, plaintext.to_vec());

        let decrypted = decrypt_bytes(&dek, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext.to_vec());
    }

    #[test]
    fn decrypting_with_the_wrong_dek_fails_clearly() {
        let dek_a: Dek = Generate::generate();
        let dek_b: Dek = Generate::generate();
        let ciphertext = encrypt_bytes(&dek_a, b"secret pool state").unwrap();

        let result = decrypt_bytes(&dek_b, &ciphertext);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("wrong key"));
    }

    #[test]
    fn decrypting_truncated_data_fails_clearly_instead_of_panicking() {
        let dek: Dek = Generate::generate();
        let result = decrypt_bytes(&dek, &[1, 2, 3]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("truncated"));
    }

    #[test]
    fn two_encryptions_of_the_same_plaintext_produce_different_ciphertext() {
        // Proves the nonce is actually fresh/random per call, not reused.
        let dek: Dek = Generate::generate();
        let a = encrypt_bytes(&dek, b"same plaintext").unwrap();
        let b = encrypt_bytes(&dek, b"same plaintext").unwrap();
        assert_ne!(a, b);
    }
}
