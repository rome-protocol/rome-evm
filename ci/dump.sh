#!/bin/bash

solana account 5yuefgbJJpmFNK2iiYbLSpv1aZXq7F9AUKkZKErTYCvs --output-file sol_usdc_meteora_pool.json 	--output json-compact --url mainnet-beta
solana account So11111111111111111111111111111111111111112  --output-file wsol_mint.json 				--output json-compact --url mainnet-beta
solana account 3ESUFCnRNgZ7Mn2mPPUMmXYaKU8jpnV9VtA17M7t2mHQ --output-file usdc_vault.json 				--output json-compact --url mainnet-beta
solana account FERjPVNEa7Udq8CEv68h6tPL46Tq7ieE49HrE2wea3XT --output-file wsol_vault.json 				--output json-compact --url mainnet-beta
solana account 3RpEekjLE5cdcG15YcXJUpxSepemvq2FpmMcgo342BwC --output-file a_vault_lp_mint.json 			--output json-compact --url mainnet-beta
solana account FZN7QZ8ZUUAxMPfxYEYkH3cXUASzH8EqA6B4tyCL8f1j --output-file b_vault_lp_mint.json 			--output json-compact --url mainnet-beta
solana account CNc2A5yjKUa9Rp3CVYXF9By1qvRHXMncK9S254MS9JeV --output-file a_vault_lp.json 				--output json-compact --url mainnet-beta
solana account 7LHUMZd12RuanSXhXjQWPSXS6QEVQimgwxde6xYTJuA7 --output-file b_vault_lp.json 				--output json-compact --url mainnet-beta

solana account GvDMxPzN1sCj7L26YDK2HnMRXEQmQ2aemov8YBtPS7vR --output-file switchboard_sol_usd.json 		--output json-compact --url mainnet-beta

