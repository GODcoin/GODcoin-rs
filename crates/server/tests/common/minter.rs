use super::create_tx_header;
use actix::prelude::*;
use godcoin::{blockchain::GenesisBlockInfo, prelude::*};
use godcoin_server::{handle_request, prelude::*, ServerData};
use sodiumoxide::randombytes;
use std::{env, fs, path::PathBuf, sync::Arc};

pub struct TestMinter(ServerData, GenesisBlockInfo, PathBuf);

impl TestMinter {
    pub fn new() -> Self {
        godcoin::init().unwrap();
        let mut tmp_dir = env::temp_dir();
        {
            let mut s = String::from("godcoin_test_");
            let mut num: [u8; 8] = [0; 8];
            randombytes::randombytes_into(&mut num);
            s.push_str(&format!("{}", u64::from_be_bytes(num)));
            tmp_dir.push(s);
        }
        fs::create_dir(&tmp_dir).expect(&format!("Could not create temp dir {:?}", &tmp_dir));

        let chain = Arc::new(Blockchain::new(&tmp_dir));
        let minter_key = KeyPair::gen();
        let info = chain.create_genesis_block(minter_key.clone());

        {
            let txs = {
                let mut txs = Vec::with_capacity(1);

                let mut tx = MintTx {
                    base: create_tx_header(TxType::MINT, "0.0000 GRAEL"),
                    to: (&info.script).into(),
                    amount: "1000.0000 GRAEL".parse().unwrap(),
                    script: info.script.clone(),
                };

                tx.append_sign(&info.wallet_keys[1]);
                tx.append_sign(&info.wallet_keys[0]);

                let tx = TxVariant::MintTx(tx);
                txs.push(tx);

                txs.push(TxVariant::RewardTx(RewardTx {
                    base: Tx {
                        tx_type: TxType::REWARD,
                        fee: "0.0000 GRAEL".parse().unwrap(),
                        timestamp: 0,
                        signature_pairs: Vec::new(),
                    },
                    to: (&info.script).into(),
                    rewards: Asset::default(),
                }));
                txs
            };

            let head = chain.get_chain_head();
            let child = head.new_child(txs).sign(&info.minter_key);
            chain.insert_block(child).unwrap();
        }

        let minter = Minter::new(Arc::clone(&chain), minter_key, (&info.script).into()).start();
        let data = ServerData { chain, minter };
        Self(data, info, tmp_dir)
    }

    pub fn chain(&self) -> &Blockchain {
        &self.0.chain
    }

    pub fn genesis_info(&self) -> &GenesisBlockInfo {
        &self.1
    }

    pub fn produce_block(&self) -> impl Future<Item = Result<(), verify::BlockErr>, Error = ()> {
        self.0
            .minter
            .send(ForceProduceBlock)
            .map_err(|e| panic!("{}", e))
    }

    pub fn request(&self, req: MsgRequest) -> impl Future<Item = MsgResponse, Error = ()> {
        handle_request(&self.0, req)
    }
}

impl Drop for TestMinter {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.2).expect("Failed to rm dir");
    }
}
