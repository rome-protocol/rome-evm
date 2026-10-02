use {
    crate::{
        error::{Result, RomeProgramError::*}, U256, H160,
    },
    solana_program::{
        pubkey::Pubkey, instruction::AccountMeta, program_error::ProgramError,
    },
    std::{
        convert::{TryFrom, TryInto}
    }
};

#[macro_export]
macro_rules! len_eq {
    ($abi:ident, $len:expr) => {
        if $abi.len() != $len {
            return Err(InvalidNonEvmInstructionData)
        }
    }
}
#[macro_export]
macro_rules! len_ge {
    ($abi:ident, $exp:expr) => {
        if $abi.len() < $exp || $exp == 0 {
            return Err(InvalidNonEvmInstructionData)
        }
    }
}

#[macro_export]
macro_rules! val_eq {
    ($var:ident, $val:expr) => {
        if $var != $val {
            return Err(InvalidNonEvmInstructionData)
        }
    }
}
pub use len_eq;
pub use len_ge;
use crate::handler::CallInterrupt;

pub fn next<'a, I: Iterator<Item = &'a AccountMeta>>(iter: &mut I) -> Result<Pubkey> {
    iter
        .next()
        .map(|x| x.pubkey)
        .ok_or(ProgramError::NotEnoughAccountKeys.into())
}

pub fn get_pubkey(src: &[u8]) -> Result<(Pubkey, &[u8])> {
    len_ge!(src, 32);
    let (left, right) = src.split_at(32);
    let key = Pubkey::try_from(left).unwrap();

    Ok((key, right))
}
pub fn get_h160(src: &[u8]) -> Result<(H160, &[u8])> {
    len_ge!(src, 32);
    let (left, right) = src.split_at(32);

    if left[..12].iter().any(|&a| a != 0) {
        return Err(ParseNonEvmInstructionDataError("cannot convert abi word to h160 due to non-zero padding".to_string()))
    }
    let addr = H160::from_slice(&left[12..]);

    Ok((addr, right))
}
pub fn get_u64(src: &[u8]) -> Result<(u64, &[u8])> {
    len_ge!(src, 32);
    let (left, right) = src.split_at(32);
    let v = U256::from_big_endian(left);

    let &U256(ref arr) = &v;
    if !fits_word(arr) {
        return Err(ParseNonEvmInstructionDataError("cannot convert abi word to u64 due to non-zero padding".to_string()))
    }

    Ok((arr[0], right))
}
fn fits_word(arr: &[u64; 4]) -> bool {
    for i in 1..4 {
        if arr[i] != 0 {
            return false;
        }
    }

    true
}
pub fn get_usize(src: &[u8], offset: usize) -> Result<usize> {
    let end = offset.checked_add(32).ok_or(InvalidNonEvmInstructionData)?;
    len_ge!(src, end);
    let v = U256::from_big_endian(&src[offset..end]);

    let &U256(ref arr) = &v;
    if !fits_word(arr) || arr[0] > usize::MAX as u64 {
        return Err(ParseNonEvmInstructionDataError("cannot convert U256 to usize".to_string()))
    }

    Ok(arr[0] as usize)
}
pub fn get_u256(src: &[u8]) -> Result<U256> {
    len_eq!(src, 32);
    Ok(U256::from_big_endian(src))
}

pub fn get_u8_unchecked(slice: &[u8]) -> Result<u8> {
    U256::from_big_endian(slice)
        .try_into()
        .map_err(|_| InvalidNonEvmInstructionData)
}


// 0000000000000000000000000000000000000000000000000000000000000040     offset
// 0000000000000000000000000000000000000000000000000000000000000003     len

// 0000000000000000000000000000000000000000000000000000000000000060     offset_item_1 from this
// 00000000000000000000000000000000000000000000000000000000000000c0     offset_item_2
// 0000000000000000000000000000000000000000000000000000000000000120     offset_item_3

pub fn offsets_slice_of_slices(abi: &[u8], pos: usize) -> Result<Vec<usize>> {
    let mut offset = get_usize(abi, pos)?;
    let len = get_usize(abi, offset)?;

    offset += 32;
    let ref_point = offset;

    let mut vec = vec![];
    for _ in 0..len {
        let item = ref_point.checked_add(get_usize(abi, offset)?).ok_or(InvalidNonEvmInstructionData)?;
        vec.push(item);
        offset += 32;
    }

    Ok(vec)
}


pub fn get_slice(abi: &[u8], pos: usize, element_size: usize) -> Result<&[u8]> {
    let mut offset = get_usize(abi, pos)?;

    let len = get_usize(abi, offset)?;
    let size = len.checked_mul(element_size).ok_or(InvalidNonEvmInstructionData)?;
    offset += 32;

    let end = offset.checked_add(size).ok_or(InvalidNonEvmInstructionData)?;
    len_ge!(abi, end);
    let slice = &abi[offset .. end];

    Ok(slice)
}
// 0000000000000000000000000000000000000000000000000000000000000020
// 0000000000000000000000000000000000000000000000000000000000000004
// e903000000000000000000000000000000000000000000000000000000000000
pub fn get_nested_slice(abi: &[u8], pos: usize) -> Result<&[u8]> {
    let mut offset = get_usize(abi, pos)?.checked_add(pos).ok_or(InvalidNonEvmInstructionData)?;

    let len = get_usize(abi, offset)?;
    offset += 32;

    let end = offset.checked_add(len).ok_or(InvalidNonEvmInstructionData)?;
    len_ge!(abi, end);
    let slice = &abi[offset .. end];

    Ok(slice)
}
pub fn split_slice_of_slices(abi: &[u8], offset: usize) -> Result<Vec<&[u8]>> {
    let offsets = offsets_slice_of_slices(abi, offset)?;

    offsets
        .iter()
        .map(|pos| get_nested_slice(abi, *pos))
        .collect::<Result<Vec<&[u8]>>>()
}
pub fn align_len(len: usize) -> usize {
    let aligned = 32 + 32 + len;

    if aligned % 32 != 0 {
        (aligned / 32 + 1) * 32
    } else {
        aligned
    }
}
pub fn slice_to_abi(msg: &[u8], offset: usize) -> Vec<u8> {
    let len = align_len(msg.len());
    let mut abi = vec![0; len];

    let (left, right) = abi.split_at_mut(32);
    let x: U256 = (offset + 32).into();
    x.to_big_endian(left);

    let (left, right) = right.split_at_mut(32);
    let x: U256 = msg.len().into();
    x.to_big_endian(left);

    let (left, _) = right.split_at_mut(msg.len());

    left.copy_from_slice(msg);

    abi
}
pub fn  add_dyn_to_static(mut static_src: Vec<u8>, dynamic_val: &[u8]) -> Vec<u8> {
    assert_eq!(static_src.len() % 32, 0);  // aligned

    let len = align_len(dynamic_val.len());
    let mut abi = vec![0; len];

    let (left, right) = abi.split_at_mut(32);
    let x: U256 = (static_src.len() + 32).into();
    x.to_big_endian(left);

    let (left, right) = right.split_at_mut(32);
    let x: U256 = dynamic_val.len().into();
    x.to_big_endian(left);

    let (left, _) = right.split_at_mut(dynamic_val.len());

    left.copy_from_slice(&dynamic_val);
    static_src.append(&mut abi);

    static_src
}

pub fn u64_to_bytes32(x: u64, dst: &mut [u8; 32]) {
    let val: U256 = x.into();
    val.to_big_endian(dst);
}

pub fn not_payable(params: &CallInterrupt) -> Result<()> {
    if let Some(t) = params.transfer {
        if !t.value.is_zero()  {
            return Err(NonEvmCallError("TransferProhibited".to_string()))
        }
    }

    Ok(())
}

pub fn get_account_meta(abi: &[u8], pos: usize) -> Result<Vec<AccountMeta>> {
    let slice = get_slice(abi, pos, 32 * 3)?;

    let mut meta = vec![];
    for i in (0..slice.len()).step_by(32*3) {
        let pubkey = Pubkey::try_from(&slice[i .. i + 32]).unwrap();
        let is_signer  = get_u8_unchecked(&slice[i + 32 .. i + 64])? != 0;
        let is_writable = get_u8_unchecked(&slice[i + 64 .. i + 96])? != 0 ;
        meta.push(AccountMeta { pubkey, is_signer, is_writable });
    }

    Ok(meta)
}

pub fn get_vec_bytes32(abi: &[u8], pos: usize) -> Result<Vec<&[u8]>> {
    let slice = get_slice(abi, pos, 32)?;
    let vec = slice.chunks(32).into_iter().collect::<Vec<_>>();
    Ok(vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(n: u64) -> [u8; 32] {
        let mut w = [0_u8; 32];
        w[24..].copy_from_slice(&n.to_be_bytes());
        w
    }

    fn abi(words: &[[u8; 32]]) -> Vec<u8> {
        words.concat()
    }

    #[test]
    fn get_slice_returns_the_addressed_elements() {
        let src = abi(&[word(32), word(2), [0xaa; 32], [0xbb; 32]]);
        let mut expected = vec![0xaa; 32];
        expected.extend_from_slice(&[0xbb; 32]);
        assert_eq!(get_slice(&src, 0, 32).unwrap(), expected.as_slice());
    }

    #[test]
    fn get_slice_rejects_an_offset_that_wraps() {
        let src = abi(&[word(u64::MAX - 30), word(0)]);
        assert!(matches!(get_slice(&src, 0, 32), Err(InvalidNonEvmInstructionData)));
    }

    #[test]
    fn get_slice_rejects_a_length_that_wraps() {
        let src = abi(&[word(32), word(u64::MAX / 16 + 1)]);
        assert!(matches!(get_slice(&src, 0, 32), Err(InvalidNonEvmInstructionData)));
    }

    #[test]
    fn get_slice_rejects_an_end_that_wraps() {
        let src = abi(&[word(32), word(u64::MAX / 32)]);
        assert!(matches!(get_slice(&src, 0, 32), Err(InvalidNonEvmInstructionData)));
    }

    #[test]
    fn get_nested_slice_rejects_an_offset_that_wraps() {
        let src = abi(&[word(0), word(u64::MAX - 31)]);
        assert!(matches!(get_nested_slice(&src, 32), Err(InvalidNonEvmInstructionData)));
    }

    #[test]
    fn get_nested_slice_rejects_a_length_that_wraps() {
        let src = abi(&[word(32), word(u64::MAX - 40)]);
        assert!(matches!(get_nested_slice(&src, 0), Err(InvalidNonEvmInstructionData)));
    }

    #[test]
    fn offsets_slice_of_slices_rejects_an_element_offset_that_wraps() {
        let src = abi(&[word(32), word(1), word(u64::MAX - 60)]);
        assert!(matches!(offsets_slice_of_slices(&src, 0), Err(InvalidNonEvmInstructionData)));
    }
}
