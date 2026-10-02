// SPDX-License-Identifier: LicenseRef-Rome-Protocol
// Reference/example contract — NOT audited, NOT for production use.
pragma solidity ^0.8.20;

import "./interface.sol";
import "./shared.sol";

contract HelloWorldSolanaProgram is Shared {
    bytes constant PROGRAM_ID = "AwsrRzgubeCsP77GNRdGCH4XefWkczFPrAzm86q7gVv4";

    function hello_world() public {
        bytes32 program_id = SystemProgram.base58_to_bytes32(PROGRAM_ID);

        ICrossProgramInvocation.AccountMeta[] memory accounts = new ICrossProgramInvocation.AccountMeta[](2);
        accounts[0] = ICrossProgramInvocation.AccountMeta(signer_pda(), true, false);
        accounts[1] = ICrossProgramInvocation.AccountMeta(SystemProgram.payer(), true, true);

        bytes memory data = hex"ffffff"; 

        CrossProgramInvocation.invoke_signed(program_id, accounts, data);
    }

    function signer_pda() public view returns (bytes32) {
        bytes32 rome_program = SystemProgram.rome_evm_program_id();
        ISystemProgram.Seed[] memory pda_seeds = Shared.balance_key_seeds(msg.sender, block.chainid);
        (bytes32 key,) = SystemProgram.find_program_address(rome_program, pda_seeds);

        return key;
    }
}

