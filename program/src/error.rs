use {
    crate::{accounts::LockType, AccountType},
    evm::{H160, H256, U256},
    rlp::DecoderError,
    solana_program::{
        instruction::InstructionError,
        program_error::ProgramError,
        pubkey::{ParsePubkeyError, Pubkey, PubkeyError},
    },
    solana_precompile_error::PrecompileError,
    thiserror::Error,
};

#[cfg(not(target_os = "solana"))]
use solana_client::client_error::{ClientError, ClientErrorKind};

pub type Result<T> = std::result::Result<T, RomeProgramError>;

pub type ErrBox = Box<dyn std::error::Error>;

#[derive(Debug, Error)]
pub enum RomeProgramError {
    #[error("Signer not found, or more than one signer was found")]
    InvalidSigner,

    #[error("The AccountInfo parser expected a Sysvar, but the key was invalid: {0}")]
    InvalidSysvar(Pubkey),

    #[error("The AccountInfo has an invalid owner: {0}")]
    InvalidOwner(Pubkey),

    #[error("An IO error was captured, wrap it up and forward it along {0}")]
    IoError(std::io::Error),

    #[error("An solana program error: {0}")]
    ProgramError(ProgramError),

    #[error("An instruction that wasn't recognised was sent")]
    UnknownInstruction(u8),

    #[error("Custom error: {0}")]
    Custom(String),

    #[error("User does not have sufficient funds (Wei): {0} {0}")]
    InsufficientFunds(H160, U256),

    #[error("Payer does not have sufficient lamports: {0} {1}")]
    InsufficientLamports(Pubkey, u64),

    #[error("RLP Decored error: {0}")]
    RlpDecoderError(#[from] DecoderError),

    #[error("Invalid account type: {0}")]
    InvalidAccountType(Pubkey),

    #[error("Invalid hash of the tx in the holder account: {0}")]
    InvalidHolderHash(Pubkey),

    #[error("Invalid data length: {0} {1}, {2}")]
    InvalidDataLength(Pubkey, usize, usize),

    #[error("Static Mode Violation: {0}")]
    StaticModeViolation(H160),

    #[error("Deploy a contract to an existing account: {0}")]
    DeployContractToExistingAccount(H160),

    #[error("Calculation overflow")]
    CalculationOverflow,

    #[error("An solana Pubkey error: {0}")]
    PubkeyError(#[from] PubkeyError),

    #[error("account not found: {0}")]
    AccountNotFound(Pubkey),

    #[error("PDA account not found: {0} account type {1:?}")]
    PdaAccountNotFound(Pubkey, AccountType),

    #[error("PDA untyped account not found: {0}")]
    PdaUntypedAccountNotFound(Pubkey),

    #[error("PDA untyped account is not created: {0}")]
    PdaUntypedAccountNotCreated(Pubkey),

    #[error("attempt to init an initialized account: {0}")]
    AccountInitialized(Pubkey),

    #[error("Invalid Ethereum transaction signature: {0}")]
    InvalidEthereumSignature(String),

    #[error("Invalid instruction data")]
    InvalidInstructionData,

    #[error("Invalid non-EVM instruction data")]
    InvalidNonEvmInstructionData,

    #[error("Error to parse non-EVM instruction data: {0}")]
    ParseNonEvmInstructionDataError(String),

    #[error("Cannot deserialize non-evm instruction data: {0}")]
    DeserializeInstructionDataError(String),

    #[cfg(not(target_os = "solana"))]
    #[error("rpc client error {0:?}")]
    RpcClientError(ClientError),

    #[cfg(not(target_os = "solana"))]
    #[error("bincode error {0:?}")]
    BincodeError(bincode::error::DecodeError),

    #[error("Incorrect chain_id: {0:?} ")]
    IncorrectChainId(Option<(u64, u64)>),

    #[error("Vm fault: {0:?}")]
    VmFault(String),

    #[error("Account is locked: {0} {1:?}")]
    AccountLocked(Pubkey, Option<LockType>),

    #[error("Account lock not found: {0} {1:?}")]
    AccountLockNotFound(Pubkey, Option<LockType>),

    #[error("StateHolder's iteration cast error: {0}")]
    IterationCastError(String),

    #[error("Invalid transaction nonce for address: {0} {1} {2}")]
    InvalidTxNonce(H160, u64, u64),

    #[error("Allocation/deallocation error: {0}")]
    AllocationError(String),

    #[error("the feature is unimplemented: {0} ")]
    Unimplemented(String),

    #[error("Iterative transaction is finished: {0}")]
    UnnecessaryIteration(H256),

    #[error("parse Pubkey error: {0}")]
    ParsePubkeyError(#[from] ParsePubkeyError),

    #[error("Unregistered chain_id: {0} ")]
    UnregisteredChainId(u64),

    #[error("attempt to create an existing account: {0}")]
    AccountAlreadyExists(Pubkey),

    #[error("attempt to allocate an existing account: {0}")]
    AccountAlreadyInUse(Pubkey),

    #[error("attempt to transfer SOL from account with non-empty data: {0}")]
    TransferFromAccountWithData(Pubkey),

    #[error("Inconsistent account list")]
    InconsistentAccountList,

    #[error("Inconsistency between the rpl type and the rome-evm instruction type")]
    IncorrectRlpType,

    #[error("Incorrect deposit instruction parameters")]
    InvalidDepositInstruction,

    #[error("deposit/withdraw tx.value must be multiple of {0}")]
    TxValueNotMultipleOf(String),

    #[error("transaction value exceeds the u64 format")]
    TxValueExceedsU64,

    #[error("insufficient gas: {0} {1}")]
    InsufficientGas(U256, U256),

    #[error("An solana instruction error: {0}")]
    InstructionError(#[from] InstructionError),

    #[error("Spl decimals is too high {0} {1}")]
    TooHighSplDecimals(u8, Pubkey),

    #[error("Invalid or non-existing withdrawal account: {0}")]
    InvalidWithdrawalAccount(Pubkey),
    
    #[error("Invalid SPL mint account: {0}")]
    InvalidSplMintAccount(Pubkey),

    #[error("TxHolder size exceeded: {0}")]
    TxHolderSizeExceeded(Pubkey),
    
    #[error("AtomicTxFailed: {0}")]
    AtomicTxFailed(String),

    #[error("NonEvmCallError: {0}")]
    NonEvmCallError(String),

    #[error("DELEGATECALL/CALLCODE into a precompile that signs as the caller is prohibited")]
    DelegatecallOwnerAuthority,
    
    #[error("Cross-program invocation is prohibited in iterative tx")]
    CpiProhibitedInIterativeTx,

    #[error("Transaction cannot be atomic")]
    TxCannotBeAtomic,

    #[error("program account not found: {0}")]
    ProgramAccountNotFound(Pubkey),

    #[error("elf-account not found: {0}")]
    ElfAccountNotFound(Pubkey),

    #[error("ReplayProtection")]
    ReplayProtection,

    #[error("Ed25519 instruction not found")]
    Ed25519IxNotFound,

    #[error("PrecompileError: {0}")]
    PrecompileError(#[from] PrecompileError),

    #[error("unexpected ed25519 instruction index")]
    UnexpectedEd25519InstructionIndex,
    
    #[error("API instruction is prohibited to be executed on-chain")]
    InstructionProhibited,

    #[error("Invalid gas-estimate authority")]
    InvalidGasEstimateAuthority,

    #[error("Opcode limit exceeded")]
    OpcodeLimitExceeded,

    #[error("settle authorization expired: deadline passed")]
    SignatureExpired,

    #[error("settle authorization signature is not low-s canonical (EIP-2)")]
    NonCanonicalSignature,

    #[error("settle authorization recovery byte must be 27 or 28")]
    InvalidRecoveryByte,

    #[error("settle authorization signer does not match user")]
    SignerNotUser,

    // TODO: remove after removing the mollusk repo
    #[error("SimulateTransactionError: {0}")]
    SimulateTransactionError(String),
}

impl From<ProgramError> for RomeProgramError {
    fn from(e: ProgramError) -> Self {
        RomeProgramError::ProgramError(e)
    }
}

impl From<std::io::Error> for RomeProgramError {
    fn from(e: std::io::Error) -> Self {
        RomeProgramError::IoError(e)
    }
}

impl From<RomeProgramError> for ProgramError {
    fn from(err: RomeProgramError) -> ProgramError {
        match err {
            RomeProgramError::ProgramError(e) => e,
            _ => ProgramError::Custom(0),
        }
    }
}
#[cfg(not(target_os = "solana"))]
impl From<ClientError> for RomeProgramError {
    fn from(e: ClientError) -> RomeProgramError {
        RomeProgramError::RpcClientError(e)
    }
}
#[cfg(not(target_os = "solana"))]
impl From<bincode::error::DecodeError> for RomeProgramError {
    fn from(e: bincode::error::DecodeError) -> RomeProgramError {
        RomeProgramError::BincodeError(e)
    }
}

#[cfg(not(target_os = "solana"))]
impl From<RomeProgramError> for ClientError {
    fn from(e: RomeProgramError) -> ClientError {
        ClientErrorKind::Custom(e.to_string()).into()
    }
}

#[cfg(not(target_os = "solana"))]
impl From<mollusk::error::MolluskError> for RomeProgramError {
    fn from(e: mollusk::error::MolluskError) -> Self {
        use mollusk::error::MolluskError;
        match e {
            MolluskError::SimulateTransactionError(s) => Self::SimulateTransactionError(s),
            MolluskError::ProgramAccountNotFound(p) => Self::ProgramAccountNotFound(p),
            MolluskError::ElfAccountNotFound(p) => Self::ElfAccountNotFound(p),
            MolluskError::Custom(s) => Self::Custom(s),
            MolluskError::Program(pe) => Self::ProgramError(pe),
            MolluskError::Instruction(ie) => Self::InstructionError(ie),
        }
    }
}

