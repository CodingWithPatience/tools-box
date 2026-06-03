use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use pbkdf2::pbkdf2;
use rand::RngCore;

/// PBKDF2 迭代次数
const PBKDF2_ITERATIONS: u32 = 100_000;

/// Salt 长度（字节）
const SALT_LENGTH: usize = 16;

/// IV/Nonce 长度（字节）
const IV_LENGTH: usize = 12;

/// 生成随机 salt
pub fn generate_salt() -> [u8; SALT_LENGTH] {
    let mut salt = [0u8; SALT_LENGTH];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// 生成随机 IV/Nonce
fn generate_iv() -> [u8; IV_LENGTH] {
    let mut iv = [0u8; IV_LENGTH];
    OsRng.fill_bytes(&mut iv);
    iv
}

/// 使用 PBKDF2 从主密码派生 256-bit 密钥
///
/// # 参数
/// - `master_password`: 用户输入的主密码
/// - `salt`: 随机 salt
///
/// # 返回
/// 32 字节的派生密钥
pub fn derive_key(master_password: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    let _ = pbkdf2::<hmac::Hmac<sha2::Sha256>>(
        master_password.as_bytes(),
        salt,
        PBKDF2_ITERATIONS,
        &mut key,
    );
    key
}

/// 计算主密码的验证哈希
///
/// 用于验证用户输入的主密码是否正确
pub fn hash_master_password(password: &str, salt: &[u8]) -> Vec<u8> {
    let key = derive_key(password, salt);
    // 对派生密钥进行 SHA-256 哈希作为验证值
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(&key);
    hash.to_vec()
}

/// 使用 AES-256-GCM 加密密码
///
/// # 返回
/// (密文, iv) - 密文包含 authentication tag
pub fn encrypt_password(key: &[u8; 32], plaintext: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .context("无法创建 AES-GCM 密码器")?;

    let iv = generate_iv();
    let nonce = Nonce::from_slice(&iv);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("加密失败: {}", e))?;

    Ok((ciphertext, iv.to_vec()))
}

/// 使用 AES-256-GCM 解密密码
pub fn decrypt_password(key: &[u8; 32], ciphertext: &[u8], iv: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .context("无法创建 AES-GCM 密码器")?;

    let nonce = Nonce::from_slice(iv);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("解密失败: {}", e))?;

    String::from_utf8(plaintext)
        .context("解密结果不是有效的 UTF-8 文本")
}

/// 密码生成配置
pub struct PasswordConfig {
    pub length: usize,
    pub use_uppercase: bool,
    pub use_lowercase: bool,
    pub use_digits: bool,
    pub use_symbols: bool,
}

impl Default for PasswordConfig {
    fn default() -> Self {
        Self {
            length: 16,
            use_uppercase: true,
            use_lowercase: true,
            use_digits: true,
            use_symbols: true,
        }
    }
}

/// 生成随机密码
pub fn generate_password(config: &PasswordConfig) -> String {
    let mut charset = Vec::new();

    if config.use_lowercase {
        charset.extend_from_slice(b"abcdefghijklmnopqrstuvwxyz");
    }
    if config.use_uppercase {
        charset.extend_from_slice(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }
    if config.use_digits {
        charset.extend_from_slice(b"0123456789");
    }
    if config.use_symbols {
        charset.extend_from_slice(b"!@#$%^&*()_+-=[]{}|;:,.<>?");
    }

    // 如果没有选择任何字符集，默认使用小写字母
    if charset.is_empty() {
        charset.extend_from_slice(b"abcdefghijklmnopqrstuvwxyz");
    }

    let mut rng = OsRng;
    let password: String = (0..config.length)
        .map(|_| {
            let idx = rng.next_u32() as usize % charset.len();
            charset[idx] as char
        })
        .collect();

    password
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let key = derive_key("test_password", &generate_salt());
        let plaintext = "my_secret_password_123";

        let (ciphertext, iv) = encrypt_password(&key, plaintext).unwrap();
        let decrypted = decrypt_password(&key, &ciphertext, &iv).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = derive_key("password1", &generate_salt());
        let key2 = derive_key("password2", &generate_salt());

        let (ciphertext, iv) = encrypt_password(&key1, "secret").unwrap();
        let result = decrypt_password(&key2, &ciphertext, &iv);

        assert!(result.is_err());
    }

    #[test]
    fn test_generate_password() {
        let config = PasswordConfig {
            length: 20,
            use_uppercase: true,
            use_lowercase: true,
            use_digits: true,
            use_symbols: true,
        };

        let password = generate_password(&config);
        assert_eq!(password.len(), 20);
    }

    #[test]
    fn test_derive_key_deterministic() {
        let salt = generate_salt();
        let key1 = derive_key("same_password", &salt);
        let key2 = derive_key("same_password", &salt);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_hash_master_password() {
        let salt = generate_salt();
        let hash1 = hash_master_password("correct_password", &salt);
        let hash2 = hash_master_password("correct_password", &salt);
        assert_eq!(hash1, hash2);

        let hash3 = hash_master_password("wrong_password", &salt);
        assert_ne!(hash1, hash3);
    }
}
