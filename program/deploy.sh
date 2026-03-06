cargo build-sbf

solana address -k target/deploy/solana_transfer_data_program-keypair.json

solana program deploy \
    --program-id target/deploy/solana_transfer_data_program-keypair.json \
    target/deploy/solana_transfer_data_program.so
