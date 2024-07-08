use ed25519_dalek::SigningKey;
use ed25519_dalek::{Signature, Signer, Verifier};
/// Signer256k1 to generate Scp256k1 signature
pub(crate) struct SignerEd25519 {
    pub signing_key: SigningKey,
}

impl SignerEd25519 {
    // Set a new signer. Private_key is 32 bytes hex key, witout 0x prefix
    pub(crate) fn new(private_key: String) -> SignerEd25519 {
        let private_key: [u8; 32] = hex::decode(private_key).unwrap().try_into().unwrap();
        let signing_key: SigningKey = SigningKey::from_bytes(&private_key);

        SignerEd25519 { signing_key }
    }

    pub(crate) fn sign(&self, message: impl AsRef<[u8]>) -> Signature {
        self.signing_key.sign(message.as_ref())
    }

    pub(crate) fn verify(&self, message: impl AsRef<[u8]>, signature: Signature) -> bool {
        self.signing_key
            .verify(message.as_ref(), &signature)
            .is_ok()
    }
}

mod test {
    use super::Signature;
    use super::SignerEd25519;
    #[test]
    fn test() {
        let private_key_env = std::env::var("NOTARY_PRIVATE_KEY_SECP256k1").unwrap();
        println!("private_key {:}", private_key_env);
        let signer = SignerEd25519::new(private_key_env);
        println!("signing_key {:?}", signer.signing_key.as_bytes());

        let message: String = String::from("This is a test of the tsunami alert system.");
        let signature: Signature = signer.sign(message.clone());
        assert!(signer.verify(message, signature));
    }

    #[test]
    fn test_verify() {
        let private_key_env = std::env::var("NOTARY_PRIVATE_KEY_SECP256k1").unwrap();
        let signer = SignerEd25519::new(private_key_env);

        let signature =
            "8A73D7A1F3F9BD2CEB611A9FE685D785AC43F2B377AFE168CB7A644102AF1F934AB6E80761373D132AB9C1713978EEC3916B5F7C5A91952A582B8DEDCD558A01";

        let merkle_root = [
            149, 169, 221, 96, 239, 142, 48, 24, 181, 120, 87, 116, 138, 112, 141, 210, 107, 166,
            53, 220, 100, 183, 250, 22, 190, 61, 169, 236, 21, 50, 36, 171,
        ];
        let user_id = "1";

        use sha1::{Digest, Sha1};
        let mut hasher = Sha1::new();
        hasher.update(user_id.as_bytes());
        let nullifier = hasher.finalize();
        let nullifier_vec = nullifier.to_vec();

        let mut combined_bytes = nullifier_vec;
        combined_bytes.extend_from_slice(&merkle_root);

        let signature = &hex::decode(signature).expect("Failed to decode hex signature");

        let signature: &[u8; 64] = signature
            .as_slice()
            .try_into()
            .expect("Signature must be exactly 64 bytes");

        let signature = Signature::from_bytes(signature);
        assert!(signer.verify(combined_bytes, signature));
    }
}
