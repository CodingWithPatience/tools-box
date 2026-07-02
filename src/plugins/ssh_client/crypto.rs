use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::{Context, Result};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;

/// PBKDF2 迭代次数
const PBKDF2_ITERATIONS: u32 = 100_000;
/// 应用级密钥材料（作为 KDF 的"胡椒"，配合随机 salt 使用）
const APP_PEPPER: &[u8] = b"tools-box-ssh-client-pepper-v1";

/// 从应用密钥 + 随机盐值派生 AES-256 密钥
fn derive_key(salt: &[u8]) -> Key<Aes256Gcm> {
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(APP_PEPPER, salt, PBKDF2_ITERATIONS, &mut key);
    *Key::<Aes256Gcm>::from_slice(&key)
}

/// 加密明文密码，返回 (密文, nonce, salt)
pub fn encrypt_password(plaintext: &str) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut salt = [0u8; 32];
    OsRng.fill_bytes(&mut salt);

    let key = derive_key(&salt);
    let cipher = Aes256Gcm::new(&key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("加密失败: {}", e))?;
    Ok((ciphertext, nonce.to_vec(), salt.to_vec()))
}

/// 解密密文密码
pub fn decrypt_password(ciphertext: &[u8], iv: &[u8], salt: &[u8]) -> Result<String> {
    let key = derive_key(salt);
    let cipher = Aes256Gcm::new(&key);
    let nonce = Nonce::from_slice(iv);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("解密失败: {}", e))?;
    String::from_utf8(plaintext).context("密码解密后非有效 UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_password() {
        let plaintext = "my_secret_password123";
        let (ciphertext, iv, salt) = encrypt_password(plaintext).expect("加密应成功");
        let decrypted =
            decrypt_password(&ciphertext, &iv, &salt).expect("解密应成功");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_different_salt() {
        let plaintext = "test";
        let (_ct1, _iv1, salt1) = encrypt_password(plaintext).expect("加密应成功");
        let (_ct2, _iv2, salt2) = encrypt_password(plaintext).expect("加密应成功");
        assert_ne!(salt1, salt2, "每次加密应使用不同 salt");
    }

    #[test]
    fn test_encrypt_different_iv() {
        let plaintext = "test";
        let (_ct1, iv1, _salt1) = encrypt_password(plaintext).expect("加密应成功");
        let (_ct2, iv2, _salt2) = encrypt_password(plaintext).expect("加密应成功");
        assert_ne!(iv1, iv2, "每次加密应使用不同 nonce");
    }

    #[test]
    fn test_decrypt_wrong_iv() {
        let plaintext = "secret";
        let (ct, _iv, salt) = encrypt_password(plaintext).expect("加密应成功");
        let wrong_iv = vec![0u8; 12];
        let result = decrypt_password(&ct, &wrong_iv, &salt);
        assert!(result.is_err(), "错误 IV 应解密失败");
    }

    #[test]
    fn test_decrypt_wrong_salt() {
        let plaintext = "secret";
        let (ct, iv, _salt) = encrypt_password(plaintext).expect("加密应成功");
        let wrong_salt = vec![1u8; 32];
        let result = decrypt_password(&ct, &iv, &wrong_salt);
        assert!(result.is_err(), "错误 salt 应解密失败");
    }

    #[test]
    fn test_empty_password() {
        let (ct, iv, salt) = encrypt_password("").expect("空密码加密应成功");
        let decrypted = decrypt_password(&ct, &iv, &salt).expect("解密应成功");
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_unicode_password() {
        let plaintext = "密码🔑测试";
        let (ct, iv, salt) = encrypt_password(plaintext).expect("加密应成功");
        let decrypted =
            decrypt_password(&ct, &iv, &salt).expect("解密应成功");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_same_plaintext_different_ciphertext() {
        let plaintext = "password123";
        let (ct1, _iv1, salt1) = encrypt_password(plaintext).expect("加密应成功");
        let (ct2, _iv2, salt2) = encrypt_password(plaintext).expect("加密应成功");
        assert_ne!(salt1, salt2, "不同 salt");
        assert_ne!(ct1, ct2, "相同明文应产生不同密文");
    }
}
