// SPDX-License-Identifier: LicenseRef-Rome-Protocol
// Reference/example contract — NOT audited, NOT for production use.
pragma solidity =0.8.28;


import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "./interface.sol";

contract SPL_ERC20 is ERC20 {

    bytes32 public immutable mint_id;
    uint8 public immutable decimals_;

    constructor(bytes32 _mint_id, string memory name, string memory symbol, uint8 _decimals) ERC20(name, symbol) {
        ASplProgram.create_associated_token_account(address(this), _mint_id);
        SplProgram.decimals_eq(_mint_id, _decimals);
        mint_id = _mint_id;
        decimals_ = _decimals;
    }

    function mint(uint256 amount) public {
        _mint(msg.sender, amount);
        SplProgram.balance_ge(address(this), mint_id, totalSupply());
    }

    function withdraw(bytes32 to, uint256 amount) external {
        _burn(msg.sender, amount);
        SplProgram.transfer(to, mint_id, amount);
    }

    function decimals() override public view  returns (uint8) {
        return decimals_;
    }

    function mint_to(address target_wallet, uint256 amount) external {
        mint(amount);
        transfer(target_wallet, amount);
    }
}

