// SPDX-License-Identifier: LicenseRef-Rome-Protocol
// Reference/example contract — NOT audited, NOT for production use.
pragma solidity ^0.8.20;

interface IAssociatedSplToken {
    // eth_calls
    function program_id() external view returns(bytes32);
    // invokes
    function create_associated_token_account(bytes32 wallet, bytes32 mint) external returns(bytes32);
    function create_associated_token_account(address user, bytes32 mint) external;
}

interface ISplToken {
    struct Account {
        bytes32 mint;
        bytes32 owner;
        uint64 amount;
        bytes32 delegate;
        AccountState state;
        bool is_native;
        uint64 native_value;
        uint64 delegated_amount;
        bytes32 close_authority;
    }
    
    enum AccountState {
        Uninitialized,
        Initialized,
        Frozen
    }

    struct Seed{
        bytes item;
    }

    // eth_calls
    function program_id() external view returns(bytes32);
    function account_state(bytes32) external view returns(Account memory);
    function balance_ge(address caller, bytes32 mint, uint256 amount) external;
    function decimals_eq(bytes32 mint, uint8 decimals) external;
    // invokes
    function initialize_account3(bytes32 acc, bytes32 mint, bytes32 owner) external;
    //    function transfer_from(bytes32 from, bytes32 to, uint64 amount, Seed[] memory seeds) external;
    function transfer(bytes32 to, bytes32 mint, uint256 amount) external;
}

interface ISystemProgram {
    struct Seed{
        bytes item;
    }
    // eth_calls
    function program_id() external view returns(bytes32);
    function rome_evm_program_id() external view returns(bytes32);
    function find_program_address(bytes32 program, Seed[] memory seeds) external view returns (bytes32, uint8);
    function bytes32_to_base58(bytes32) external view returns(bytes memory);
    function base58_to_bytes32(bytes memory) external view returns(bytes32);
    function payer() external view returns(bytes32);
    // invokes
    function create_account(bytes32 owner, uint64 len, address user, bytes32 salt) external returns(bytes32);
    function allocate(bytes32 acc, uint64 space) external;
    function assign(bytes32 acc, bytes32 owner) external;
    function transfer(bytes32 to, uint64 amount, bytes32 salt) external;
}

interface IWithdraw {
    function withdrawal(bytes32 owner) payable external;
}

interface ICrossProgramInvocation {
    struct AccountMeta {
        bytes32 pubkey;
        bool is_signer;
        bool is_writable;
    }      
    struct Instruction {
        bytes32 program_id;
        AccountMeta[] accounts;
        bytes data;
    }
    function invoke_signed(bytes32 program_id, AccountMeta[] memory accounts, bytes memory data) external;
    function account_info(bytes32 pubkey) external view returns(bytes32, uint64, bytes32, bool, bool, bool, bytes memory);
}

address constant spl_token_address = address(0xff00000000000000000000000000000000000005);
address constant aspl_token_address = address(0xFF00000000000000000000000000000000000006);
address constant system_program_address = address(0xfF00000000000000000000000000000000000007);
address constant cpi_program_address = address(0xFF00000000000000000000000000000000000008);
address constant withdraw_address = address(0x4200000000000000000000000000000000000016);

ISplToken constant SplProgram = ISplToken(spl_token_address);
IAssociatedSplToken constant ASplProgram = IAssociatedSplToken(aspl_token_address);
ISystemProgram constant SystemProgram = ISystemProgram(system_program_address);
ICrossProgramInvocation constant CpiProgram = ICrossProgramInvocation(cpi_program_address);
IWithdraw constant Withdraw = IWithdraw(withdraw_address);


