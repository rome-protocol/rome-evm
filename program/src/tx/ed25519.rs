use {
    crate::{
        tx::{
            Base, eip1559_unsigned::Eip1559unsigned
        },
        error::{Result, RomeProgramError::*,}, H256, aux::derive_sender, H160,
    },
    solana_program::{
        pubkey::Pubkey,
    },
    solana_precompile_error::PrecompileError,
    solana_ed25519_program::{
        PUBKEY_SERIALIZED_SIZE,  SIGNATURE_OFFSETS_SERIALIZED_SIZE, SIGNATURE_OFFSETS_START,
    },
    rlp::Rlp,
    borsh::{BorshDeserialize, BorshSerialize},
};
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub struct Ed25519SignatureOffsets {
    pub signature_offset: u16,
    pub signature_instruction_index: u16,
    pub public_key_offset: u16,
    pub public_key_instruction_index: u16,
    pub message_data_offset: u16,
    pub message_data_size: u16,
    pub message_instruction_index: u16,
}
impl Eip1559unsigned {
    pub fn recovery_from_ed25519(&self, rlp: &Rlp, ed25519_data: &[u8]) -> Result<H160> {
        let hash = self.hash_unsign(rlp)?;
        let signer = Self::ed25519_signer(ed25519_data, hash)?;
        let addr = derive_sender(&signer);
        Ok(addr)
    }
    
    fn ed25519_signer(data: &[u8], hash: H256) -> Result<Pubkey> {
        if data.len() < SIGNATURE_OFFSETS_START {
            return Err(PrecompileError::InvalidInstructionDataSize.into());
        }
        let num_signatures = data[0] as usize;
        if num_signatures == 0 && data.len() > SIGNATURE_OFFSETS_START {
            return Err(PrecompileError::InvalidInstructionDataSize.into());
        }
        let expected_data_size = num_signatures
            .saturating_mul(SIGNATURE_OFFSETS_SERIALIZED_SIZE)
            .saturating_add(SIGNATURE_OFFSETS_START);
        // We do not check or use the byte at data[1]
        if data.len() < expected_data_size {
            return Err(PrecompileError::InvalidInstructionDataSize.into());
        }
        for i in 0..num_signatures {
            let start = i
                .saturating_mul(SIGNATURE_OFFSETS_SERIALIZED_SIZE)
                .saturating_add(SIGNATURE_OFFSETS_START);
            let end = start.saturating_add(SIGNATURE_OFFSETS_SERIALIZED_SIZE);

            let mut slice = &data[start..end];

            let offsets = Ed25519SignatureOffsets::deserialize(&mut slice)
                .map_err(|_| PrecompileError::InvalidDataOffsets)?;

            if offsets.message_instruction_index != u16::MAX ||
                offsets.signature_instruction_index != u16::MAX ||
                offsets.public_key_instruction_index != u16::MAX {
                return Err(UnexpectedEd25519InstructionIndex)
            }

            // Parse out pubkey
            let pubkey = Self::get_data_slice(
                data,
                offsets.public_key_offset,
                PUBKEY_SERIALIZED_SIZE,
            )?;
            let signer = Pubkey::try_from(pubkey).unwrap();

            // Parse out message
            let message = Self::get_data_slice(
                data,
                offsets.message_data_offset,
                offsets.message_data_size as usize,
            )?;
            
            if Self::message_authorizes(message, &hash) {
                #[cfg(not(target_os = "solana"))]
                Self::verify(data, &offsets, pubkey, message)?;
                return Ok(signer)
            }
        }
        
        Err(PrecompileError::InvalidSignature.into())
    }

    /// The signed message authorises `hash` in exactly one of two forms: the raw
    /// 32 bytes, or `ED25519_AUTH_MESSAGE_PREFIX` followed by its lowercase hex.
    /// Whole-message equality only — text that merely contains the hash does not.
    fn message_authorizes(message: &[u8], hash: &H256) -> bool {
        if message == hash.as_bytes() {
            return true;
        }
        let prefix = crate::config::ED25519_AUTH_MESSAGE_PREFIX;
        if message.len() != prefix.len() + 2 * H256::len_bytes() {
            return false;
        }
        let mut hex = [0u8; 2 * H256::len_bytes()];
        if hex::encode_to_slice(hash.as_bytes(), &mut hex).is_err() {
            return false;
        }
        let (head, tail) = message.split_at(prefix.len());
        head == prefix && tail == hex
    }

    #[cfg(not(target_os = "solana"))]
    fn verify(data: &[u8], offsets: &Ed25519SignatureOffsets, key: &[u8], mess: &[u8] ) -> Result<()> {
        use {
            ed25519_dalek::{ed25519::Signature, Verifier},
            solana_ed25519_program::SIGNATURE_SERIALIZED_SIZE,
        };
        // Parse out signature
        let signature = Self::get_data_slice(
            data,
            offsets.signature_offset,
            SIGNATURE_SERIALIZED_SIZE,
        )?;

        let signature =
            Signature::from_slice(signature).map_err(|_| PrecompileError::InvalidSignature)?;

        let arr: &[u8; 32] = key.try_into().map_err(|_| PrecompileError::InvalidPublicKey)?;
        let publickey = ed25519_dalek::VerifyingKey::from_bytes(arr)
            .map_err(|_| PrecompileError::InvalidPublicKey)?;

        publickey
            .verify(mess, &signature)
            .map_err(|_| PrecompileError::InvalidSignature)?;

        Ok(())
    }

    fn get_data_slice(data: &[u8], offset_start: u16, size: usize) -> Result<&[u8]> {
        let start = offset_start as usize;
        let end = start.saturating_add(size);
        if end > data.len() {
            return Err(PrecompileError::InvalidDataOffsets.into());
        }

        Ok(&data[start..end])
    }

}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::config::ED25519_AUTH_MESSAGE_PREFIX,
        ed25519_dalek::{Signer, SigningKey},
        solana_ed25519_program::new_ed25519_instruction_with_signature,
    };

    const HASH: [u8; 32] = [
        0x8f, 0x2a, 0x11, 0xc3, 0x00, 0xff, 0x7e, 0x9b, 0x45, 0x01, 0xde, 0xad, 0xbe, 0xef, 0x10,
        0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x0f, 0x1e,
        0x2d, 0x3c,
    ];

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    /// Ed25519 verifier instruction data as the wallet's `signMessage` would produce it:
    /// `signed` is what the key signs, `message` is what the instruction carries.
    fn ix_data(signed: &[u8], message: &[u8]) -> Vec<u8> {
        let key = key();
        let sig = key.sign(signed).to_bytes();
        new_ed25519_instruction_with_signature(message, &sig, &key.verifying_key().to_bytes()).data
    }

    fn signer_for(message: &[u8]) -> Result<Pubkey> {
        Eip1559unsigned::ed25519_signer(&ix_data(message, message), H256::from(HASH))
    }

    fn envelope() -> Vec<u8> {
        [ED25519_AUTH_MESSAGE_PREFIX, hex::encode(HASH).as_bytes()].concat()
    }

    #[test]
    fn raw_hash_message_is_accepted() {
        let signer = signer_for(&HASH).unwrap();
        assert_eq!(signer, Pubkey::from(key().verifying_key().to_bytes()));
    }

    #[test]
    fn readable_envelope_is_accepted() {
        let message = envelope();
        assert_eq!(message.len(), ED25519_AUTH_MESSAGE_PREFIX.len() + 64);
        assert_eq!(&message[..21], b"Rome authorization\n0x");
        let signer = signer_for(&message).unwrap();
        assert_eq!(signer, Pubkey::from(key().verifying_key().to_bytes()));
    }

    #[test]
    fn envelope_with_another_hash_is_rejected() {
        let other = hex::encode([0u8; 32]);
        let message = [ED25519_AUTH_MESSAGE_PREFIX, other.as_bytes()].concat();
        assert!(signer_for(&message).is_err());
    }

    #[test]
    fn envelope_with_trailing_bytes_is_rejected() {
        let mut message = envelope();
        message.push(b'\n');
        assert!(signer_for(&message).is_err());
        let mut message = envelope();
        message.push(0);
        assert!(signer_for(&message).is_err());
    }

    #[test]
    fn envelope_with_uppercase_hex_is_rejected() {
        let upper = hex::encode_upper(HASH);
        let message = [ED25519_AUTH_MESSAGE_PREFIX, upper.as_bytes()].concat();
        assert!(signer_for(&message).is_err());
    }

    #[test]
    fn envelope_with_short_hex_is_rejected() {
        let mut message = envelope();
        message.pop();
        assert!(signer_for(&message).is_err());
    }

    #[test]
    fn text_that_merely_contains_the_hash_is_rejected() {
        let hex = hex::encode(HASH);
        let message = format!("Sign to continue: 0x{hex}");
        assert!(signer_for(message.as_bytes()).is_err());
        let message = [b"Please ".as_slice(), &envelope()].concat();
        assert!(signer_for(&message).is_err());
        let message = format!("Rome authorization\n0x{hex} (confirm)");
        assert!(signer_for(message.as_bytes()).is_err());
    }

    #[test]
    fn message_shorter_than_the_prefix_is_rejected() {
        assert!(signer_for(b"Rome").is_err());
        assert!(signer_for(b"").is_err());
    }

    #[test]
    fn envelope_with_a_same_length_wrong_prefix_is_rejected() {
        let hex = hex::encode(HASH);
        for prefix in [
            b"Rome authorisation\n0x".as_slice(),
            b"rome authorization\n0x",
            b"Rome authorization\n0X",
        ] {
            assert_eq!(prefix.len(), ED25519_AUTH_MESSAGE_PREFIX.len());
            assert!(signer_for(&[prefix, hex.as_bytes()].concat()).is_err());
        }
    }

    #[test]
    fn prefix_alone_is_rejected() {
        assert!(signer_for(ED25519_AUTH_MESSAGE_PREFIX).is_err());
    }

    #[test]
    fn raw_hash_bytes_wrapped_in_the_prefix_are_rejected() {
        // prefix ‖ 32 raw bytes is not the envelope: the tail must be hex text
        let message = [ED25519_AUTH_MESSAGE_PREFIX, &HASH].concat();
        assert!(signer_for(&message).is_err());
    }

    #[test]
    fn raw_message_equal_to_another_hash_is_rejected() {
        assert!(signer_for(&[0x11u8; 32]).is_err());
    }

    #[test]
    fn entry_referencing_another_instruction_is_rejected() {
        // offsets record: message_instruction_index sits at bytes 14..16
        let mut data = ix_data(&HASH, &HASH);
        data[14..16].copy_from_slice(&0u16.to_le_bytes());
        assert!(matches!(
            Eip1559unsigned::ed25519_signer(&data, H256::from(HASH)),
            Err(UnexpectedEd25519InstructionIndex)
        ));
    }

    /// Two verifier entries; only the second one authorises the hash — its key
    /// is the signer. Entries are laid out as the Ed25519 program expects.
    #[test]
    fn matching_second_entry_of_a_two_entry_verifier_is_the_signer() {
        let other = SigningKey::from_bytes(&[3u8; 32]);
        let wallet = key();
        let other_message = [0x11u8; 32];
        let envelope = envelope();
        let entries: [(&SigningKey, &[u8]); 2] = [(&other, &other_message), (&wallet, &envelope)];

        let header = SIGNATURE_OFFSETS_START + 2 * SIGNATURE_OFFSETS_SERIALIZED_SIZE;
        let mut payload: Vec<u8> = Vec::new();
        let mut offsets: Vec<u8> = Vec::new();
        for (k, m) in entries {
            let pk = header + payload.len();
            payload.extend_from_slice(&k.verifying_key().to_bytes());
            let sig = header + payload.len();
            payload.extend_from_slice(&k.sign(m).to_bytes());
            let msg = header + payload.len();
            payload.extend_from_slice(m);
            let o = Ed25519SignatureOffsets {
                signature_offset: sig as u16,
                signature_instruction_index: u16::MAX,
                public_key_offset: pk as u16,
                public_key_instruction_index: u16::MAX,
                message_data_offset: msg as u16,
                message_data_size: m.len() as u16,
                message_instruction_index: u16::MAX,
            };
            BorshSerialize::serialize(&o, &mut offsets).unwrap();
        }
        let data = [vec![2u8, 0], offsets, payload].concat();

        let signer = Eip1559unsigned::ed25519_signer(&data, H256::from(HASH)).unwrap();
        assert_eq!(signer, Pubkey::from(wallet.verifying_key().to_bytes()));
        assert_ne!(signer, Pubkey::from(other.verifying_key().to_bytes()));
    }

    #[test]
    fn envelope_with_a_signature_over_other_bytes_is_rejected() {
        // right shape, but the key signed the raw hash, not the envelope bytes
        let data = ix_data(&HASH, &envelope());
        assert!(Eip1559unsigned::ed25519_signer(&data, H256::from(HASH)).is_err());
    }
}
