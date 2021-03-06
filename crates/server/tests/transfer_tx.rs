use godcoin::{constants::*, prelude::*};
use std::sync::Arc;

mod common;
pub use common::*;

#[test]
fn transfer_from_minter() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let from_bal = minter.chain().get_account(from_acc, &[]).unwrap().balance;
    let to_acc = {
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![KeyPair::gen().0],
            },
        );
        acc.balance = get_asset("4.00000 TEST");
        minter.create_account(acc, "2.00000 TEST", true)
    };
    let amount = get_asset("1.00000 TEST");

    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: from_acc,
            call_fn: 1,
            args: {
                let mut args = vec![];
                args.push_u64(to_acc.id);
                args.push_asset(amount);
                args
            },
            amount,
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
    minter.produce_block().unwrap();

    let chain = minter.chain();
    let cur_bal = chain.get_account(to_acc.id, &[]).unwrap().balance;
    // We add the balance that the account started with
    assert_eq!(cur_bal, to_acc.balance.checked_add(amount).unwrap());

    // The fee transfers back to the minter wallet in the form of a reward tx so it must not be
    // subtracted during the assertion. We also subtract the balance that the account was created
    // with.
    let cur_bal = chain.get_account(from_acc, &[]).unwrap().balance;
    assert_eq!(
        cur_bal,
        from_bal
            .checked_sub(amount)
            .unwrap()
            .checked_sub(to_acc.balance)
            .unwrap()
    );
}

#[test]
fn fail_transfer_to_nonexistent_account() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let from_bal = minter.chain().get_account(from_acc, &[]).unwrap().balance;
    let amount = get_asset("1.00000 TEST");

    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: from_acc,
            call_fn: 1,
            args: {
                let mut args = vec![];
                args.push_u64(0xFFFF);
                args.push_asset(amount);
                args
            },
            amount,
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };

    let res = minter.send_req(rpc::Request::Broadcast(tx));
    match res {
        Some(Err(net::ErrorKind::TxValidation(blockchain::TxErr::ScriptEval(eval_err)))) => {
            assert_eq!(eval_err.err, script::EvalErrKind::AccountNotFound);
        }
        _ => panic!("Unexpected response {:?}", res),
    }

    minter.produce_block().unwrap();
    let chain = minter.chain();
    let cur_bal = chain.get_account(from_acc, &[]).unwrap().balance;
    // Make sure no balances have been transferred
    assert_eq!(from_bal, cur_bal);
}

#[test]
fn transfer_from_user() {
    let minter = TestMinter::new();

    let (user_1, user_1_key) = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.balance = get_asset("4.00000 TEST");
        (minter.create_account(acc, "2.00000 TEST", true), key)
    };
    let user_2 = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            2,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.balance = get_asset("4.00000 TEST");
        minter.create_account(acc, "2.00000 TEST", true)
    };

    let res = {
        let tx = {
            let amount = get_asset("100.00000 TEST");
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: minter.genesis_info().owner_id,
                call_fn: 1,
                args: {
                    let mut args = vec![];
                    args.push_u64(user_1.id);
                    args.push_asset(amount);
                    args
                },
                amount,
                memo: vec![],
            }));
            tx.append_sign(&minter.genesis_info().wallet_keys[3]);
            tx.append_sign(&minter.genesis_info().wallet_keys[0]);
            tx
        };
        minter.send_req(rpc::Request::Broadcast(tx))
    };
    assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));

    let tx = {
        let amount = get_asset("99.00000 TEST");
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: user_1.id,
            call_fn: 0,
            args: {
                let mut args = vec![];
                args.push_u64(user_2.id);
                args.push_asset(amount);
                args
            },
            amount,
            memo: vec![],
        }));
        tx.append_sign(&user_1_key);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
    minter.produce_block().unwrap();

    // User 1 transferred all funds (except the starting balance) to user 2
    let user_1_bal = minter.chain().get_account(user_1.id, &[]).unwrap().balance;
    assert_eq!(user_1_bal, get_asset("4.00000 TEST"));

    // User 2 has all the funds received from user 1 plus the starting balance
    let user_2_bal = minter.chain().get_account(user_2.id, &[]).unwrap().balance;
    assert_eq!(user_2_bal, get_asset("103.00000 TEST"));

    let wallet_id = minter.genesis_info().owner_id;
    let wallet_bal = minter.chain().get_account(wallet_id, &[]).unwrap().balance;
    // The fee loops back to the minter in a reward transaction generated by the
    // minter block production, which leaves 901, subtract an additional 8 tokens for the created
    // accounts to get the actual balance.
    assert_eq!(wallet_bal, get_asset("893.00000 TEST"));
}

#[test]
fn invalid_fee_amt_caused_by_insufficient_balance() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let to_acc = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.balance = get_asset("4.00000 TEST");
        minter.create_account(acc, "2.00000 TEST", false)
    };

    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1001.00000 TEST"),
            from: from_acc,
            call_fn: 0,
            args: vec![],
            amount: get_asset("0.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(
        res,
        Some(Err(net::ErrorKind::TxValidation(
            blockchain::TxErr::InvalidAmount
        )))
    );
    minter.produce_block().unwrap();

    let chain = minter.chain();
    let cur_bal = chain.get_account(to_acc.id, &[]).unwrap().balance;
    // Created account starting balance
    assert_eq!(cur_bal, get_asset("4.00000 TEST"));

    let cur_bal = chain.get_account(from_acc, &[]).unwrap().balance;
    // Subtract the created account starting balance
    assert_eq!(cur_bal, get_asset("996.00000 TEST"));
}

#[test]
fn insufficient_fee() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let info = minter.chain().get_account_info(from_acc, &[]).unwrap();
    let bad_fee = info
        .total_fee()
        .unwrap()
        .checked_sub(get_asset("0.00001 TEST"))
        .unwrap();
    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header(&bad_fee.to_string()),
            from: from_acc,
            call_fn: 0,
            args: vec![],
            amount: get_asset("0.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(
        res,
        Some(Err(net::ErrorKind::TxValidation(
            blockchain::TxErr::InvalidFeeAmount
        )))
    );
}

#[test]
fn negative_fee_should_fail() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("-100.00000 TEST"),
            from: from_acc,
            call_fn: 0,
            args: vec![],
            amount: get_asset("0.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(
        res,
        Some(Err(net::ErrorKind::TxValidation(
            blockchain::TxErr::InvalidFeeAmount
        )))
    );
}

#[test]
fn invalid_amt_caused_by_insufficient_balance() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let to_acc = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.balance = get_asset("4.00000 TEST");
        minter.create_account(acc, "2.00000 TEST", false)
    };
    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: from_acc,
            call_fn: 0,
            args: vec![],
            amount: get_asset("500000.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(
        res,
        Some(Err(net::ErrorKind::TxValidation(
            blockchain::TxErr::InvalidAmount
        )))
    );
    minter.produce_block().unwrap();

    let chain = minter.chain();
    let cur_bal = chain.get_account(to_acc.id, &[]).unwrap().balance;
    // Created account starting balance
    assert_eq!(cur_bal, get_asset("4.00000 TEST"));

    let cur_bal = chain.get_account(from_acc, &[]).unwrap().balance;
    // Subtract the created account starting balance
    assert_eq!(cur_bal, get_asset("996.00000 TEST"));
}

#[test]
fn invalid_amt_caused_by_negative_amt() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let to_acc = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.balance = get_asset("4.00000 TEST");
        minter.create_account(acc, "2.00000 TEST", false)
    };

    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: from_acc,
            call_fn: 0,
            args: vec![],
            amount: get_asset("-500000.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(
        res,
        Some(Err(net::ErrorKind::TxValidation(
            blockchain::TxErr::InvalidAmount
        )))
    );
    minter.produce_block().unwrap();

    let chain = minter.chain();
    let cur_bal = chain.get_account(to_acc.id, &[]).unwrap().balance;
    // Created account starting balance
    assert_eq!(cur_bal, get_asset("4.00000 TEST"));

    let cur_bal = chain.get_account(from_acc, &[]).unwrap().balance;
    // Subtract the created account starting balance
    assert_eq!(cur_bal, get_asset("996.00000 TEST"));
}

#[test]
fn memo_too_large() {
    let minter = TestMinter::new();

    let from_acc = minter.genesis_info().owner_id;
    let to_acc = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.balance = get_asset("4.00000 TEST");
        minter.create_account(acc, "2.00000 TEST", false)
    };

    let tx = {
        let amount = get_asset("1.00000 TEST");
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: from_acc,
            call_fn: 1,
            args: {
                let mut args = vec![];
                args.push_u64(to_acc.id);
                args.push_asset(amount);
                args
            },
            amount,
            memo: (0..=godcoin::constants::MAX_MEMO_BYTE_SIZE)
                .map(|_| 0)
                .collect(),
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx));
    assert_eq!(
        res,
        Some(Err(net::ErrorKind::TxValidation(
            blockchain::TxErr::TxTooLarge
        )))
    );
    minter.produce_block().unwrap();

    let chain = minter.chain();
    let cur_bal = chain.get_account(to_acc.id, &[]).unwrap().balance;
    // Created account starting balance
    assert_eq!(cur_bal, get_asset("4.00000 TEST"));

    let cur_bal = chain.get_account(from_acc, &[]).unwrap().balance;
    // Subtract the created account starting balance
    assert_eq!(cur_bal, get_asset("996.00000 TEST"));
}

#[test]
fn tx_acc_dynamic_fee_increase_in_pool() {
    let minter = TestMinter::new();
    let from_acc = minter.genesis_info().owner_id;

    let res = minter
        .send_req(rpc::Request::GetAccountInfo(from_acc))
        .unwrap();
    let acc_info = match res {
        Ok(rpc::Response::GetAccountInfo(info)) => info,
        unexp @ _ => panic!("Expected GetAccountInfo response: {:?}", unexp),
    };

    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header(&acc_info.total_fee().unwrap().to_string()),
            from: from_acc,
            call_fn: 0,
            args: vec![],
            amount: Asset::new(0),
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    let res = minter.send_req(rpc::Request::Broadcast(tx)).unwrap();
    assert_eq!(res, Ok(rpc::Response::Broadcast));

    let res = minter
        .send_req(rpc::Request::GetAccountInfo(from_acc))
        .unwrap();
    let old_acc_info = acc_info;
    let acc_info = match res {
        Ok(rpc::Response::GetAccountInfo(info)) => info,
        unexp @ _ => panic!("Expected GetAccountInfo response: {:?}", unexp),
    };

    assert!(acc_info.account_fee > old_acc_info.account_fee);

    // Transaction count always start at 1, so test it as though three transactions
    // were made (this includes the create account transaction).
    let expected_fee = GRAEL_FEE_MIN
        .checked_mul(GRAEL_FEE_MULT.checked_pow(3).unwrap())
        .unwrap();
    assert_eq!(acc_info.account_fee, expected_fee);

    minter.produce_block().unwrap();
    let res = minter
        .send_req(rpc::Request::GetAccountInfo(from_acc))
        .unwrap();
    let acc_info = match res {
        Ok(rpc::Response::GetAccountInfo(info)) => info,
        unexp @ _ => panic!("Expected GetAccountInfo response: {:?}", unexp),
    };
    assert_eq!(acc_info.account_fee, expected_fee);
}

#[test]
fn tx_acc_dynamic_fee_increase() {
    let minter = Arc::new(TestMinter::new());
    let from_acc = minter.genesis_info().owner_id;

    for num in 1..10 {
        let res = minter
            .send_req(rpc::Request::GetAccountInfo(from_acc))
            .unwrap();
        let acc_info = match res {
            Ok(rpc::Response::GetAccountInfo(info)) => info,
            unexp @ _ => panic!("Expected GetAccountInfo response: {:?}", unexp),
        };

        // Add one to the transaction count since we need to include the create account transaction.
        let expected_fee = GRAEL_FEE_MIN
            .checked_mul(GRAEL_FEE_MULT.checked_pow(num + 1).unwrap())
            .unwrap();
        assert_eq!(acc_info.account_fee, expected_fee);

        let tx = {
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header(&acc_info.total_fee().unwrap().to_string()),
                from: from_acc,
                call_fn: 0,
                args: vec![],
                amount: Asset::new(0),
                memo: vec![],
            }));
            tx.append_sign(&minter.genesis_info().wallet_keys[3]);
            tx.append_sign(&minter.genesis_info().wallet_keys[0]);
            tx
        };

        let res = minter.send_req(rpc::Request::Broadcast(tx)).unwrap();
        assert_eq!(res, Ok(rpc::Response::Broadcast));
        minter.produce_block().unwrap();
    }

    for _ in 0..=FEE_RESET_WINDOW {
        minter.produce_block().unwrap();
    }

    let res = minter
        .send_req(rpc::Request::GetAccountInfo(from_acc.clone()))
        .unwrap();
    let acc_info = match res {
        Ok(rpc::Response::GetAccountInfo(info)) => info,
        unexp @ _ => panic!("Expected GetAccountInfo response: {:?}", unexp),
    };

    // Test the delta reset for fees
    let expected_fee = GRAEL_FEE_MIN.checked_mul(GRAEL_FEE_MULT).unwrap();
    assert_eq!(acc_info.account_fee, expected_fee);
}

#[test]
fn net_fee_dynamic_increase() {
    let minter = Arc::new(TestMinter::new());
    let from_acc = minter.genesis_info().owner_id;
    {
        // Create enough funds for all the accounts being created in quick succession
        let mut tx = TxVariant::V0(TxVariantV0::MintTx(MintTx {
            base: create_tx_header("0.00000 TEST"),
            to: from_acc,
            amount: get_asset("5000.00000 TEST"),
            attachment: vec![],
            attachment_name: "".to_string(),
        }));

        tx.append_sign(&minter.genesis_info().wallet_keys[1]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);

        let res = minter.send_req(rpc::Request::Broadcast(tx));
        assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
        minter.produce_block().unwrap();
    }

    let accs = Arc::new(
        (1..100)
            .map(|id| {
                let key = KeyPair::gen();
                let mut acc = Account::create_default(
                    id,
                    Permissions {
                        threshold: 1,
                        keys: vec![key.0.clone()],
                    },
                );
                let owner_acc = minter.minter().get_account_info(from_acc).unwrap();
                let fee = owner_acc
                    .total_fee()
                    .unwrap()
                    .checked_mul(GRAEL_ACC_CREATE_FEE_MULT)
                    .unwrap();
                acc.balance = fee.checked_mul(GRAEL_ACC_CREATE_MIN_BAL_MULT).unwrap();

                // Minor optimization to reduce the number of blocks being produced
                if acc.balance.amount > 500_00000 {
                    // Reset the fee window
                    for _ in 0..=NETWORK_FEE_AVG_WINDOW {
                        minter.produce_block().unwrap();
                    }
                }
                (minter.create_account(acc, &fee.to_string(), false), key)
            })
            .collect::<Vec<_>>(),
    );

    for (acc, _) in accs.as_ref() {
        let tx = {
            let amount = Asset::new(100000);
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: from_acc,
                call_fn: 1,
                args: {
                    let mut args = vec![];
                    args.push_u64(acc.id);
                    args.push_asset(amount);
                    args
                },
                amount,
                memo: vec![],
            }));
            tx.append_sign(&minter.genesis_info().wallet_keys[3]);
            tx.append_sign(&minter.genesis_info().wallet_keys[0]);
            tx
        };

        let req = rpc::Request::Broadcast(tx);
        let res = minter.send_req(req.clone()).unwrap();
        let exp = Err(net::ErrorKind::TxValidation(
            blockchain::TxErr::InvalidFeeAmount,
        ));
        if res == exp {
            for _ in 0..=FEE_RESET_WINDOW {
                minter.produce_block().unwrap();
            }
            let res = minter.send_req(req).unwrap();
            assert_eq!(res, Ok(rpc::Response::Broadcast));
        } else {
            assert_eq!(res, Ok(rpc::Response::Broadcast));
        }
    }

    for (acc, key) in accs.as_ref() {
        let tx = {
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: acc.id,
                call_fn: 0,
                args: {
                    let mut args = vec![];
                    args.push_u64(from_acc);
                    args.push_asset(Asset::default());
                    args
                },
                amount: Asset::default(),
                memo: vec![],
            }));
            tx.append_sign(&key);
            tx
        };

        let res = minter.send_req(rpc::Request::Broadcast(tx));
        assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
    }

    // Ensure the network fee gets updated
    for _ in 0..5 {
        minter.produce_block().unwrap();
    }

    {
        let res = minter.send_req(rpc::Request::GetProperties).unwrap();
        let props = match res {
            Ok(rpc::Response::GetProperties(props)) => props,
            unexp @ _ => panic!("Expected GetProperties response: {:?}", unexp),
        };

        let chain = minter.chain();
        let max_height = props.height - (props.height % 5);
        let min_height = max_height - NETWORK_FEE_AVG_WINDOW;
        assert!(min_height < max_height);

        let receipt_count = (min_height..=max_height).fold(1u64, |receipt_count, height| {
            let block = chain.get_block(height).unwrap();
            receipt_count + block.receipts().len() as u64
        });
        let receipt_count = (receipt_count / NETWORK_FEE_AVG_WINDOW) as u16;
        assert!(receipt_count > 10);

        let fee = GRAEL_FEE_MIN.checked_mul(GRAEL_FEE_NET_MULT.checked_pow(receipt_count).unwrap());
        assert_eq!(Some(props.network_fee), fee);
    }

    for _ in 0..=NETWORK_FEE_AVG_WINDOW {
        minter.produce_block().unwrap();
    }

    // Test network delta fee reset
    let expected_fee = GRAEL_FEE_MIN.checked_mul(GRAEL_FEE_NET_MULT);
    assert_eq!(minter.chain().get_network_fee(), expected_fee);
}

#[test]
fn fail_transfer_to_destroyed_account() {
    let minter = TestMinter::new();
    let owner_acc = minter.genesis_info().owner_id;

    // (1) Create the account
    let (to_acc, to_acc_key) = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.script = script::Builder::new()
            .push(
                script::FnBuilder::new(0, OpFrame::OpDefine(vec![]))
                    .push(OpFrame::AccountId(owner_acc))
                    .push(OpFrame::OpDestroy),
            )
            .build()
            .unwrap();
        acc.balance = get_asset("4.00000 TEST");
        (
            minter.create_account(acc.clone(), "2.00000 TEST", true),
            key,
        )
    };

    // (2) destroy the account
    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: to_acc.id,
            call_fn: 0,
            args: vec![],
            amount: get_asset("0.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&to_acc_key);
        tx
    };
    {
        let res = minter.send_req(rpc::Request::Broadcast(tx));
        assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
    }

    // (3) Test cannot transfer to a destroyed account (in the same block)
    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: owner_acc,
            call_fn: 1,
            args: {
                let mut args = vec![];
                args.push_u64(to_acc.id);
                args.push_asset(get_asset("1.00000 TEST"));
                args
            },
            amount: get_asset("1.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&minter.genesis_info().wallet_keys[3]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);
        tx
    };
    {
        let res = minter.send_req(rpc::Request::Broadcast(tx.clone()));
        match res {
            Some(Err(net::ErrorKind::TxValidation(blockchain::TxErr::ScriptEval(
                script::EvalErr {
                    err: script::EvalErrKind::AccountNotFound,
                    ..
                },
            )))) => {}
            _ => panic!("Unexpected response: {:?}", res),
        }
    }

    // (4) Test cannot transfer to a destroyed account (in different blocks)
    minter.produce_block().unwrap();
    {
        let res = minter.send_req(rpc::Request::Broadcast(tx));
        match res {
            Some(Err(net::ErrorKind::TxValidation(blockchain::TxErr::ScriptEval(
                script::EvalErr {
                    err: script::EvalErrKind::AccountNotFound,
                    ..
                },
            )))) => {}
            _ => panic!("Unexpected response: {:?}", res),
        }
    }
}

#[test]
fn fail_destroy_and_send_funds_to_self() {
    let minter = TestMinter::new();

    // (1) Create the account
    let (to_acc, to_acc_key) = {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            1,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.script = script::Builder::new()
            .push(
                script::FnBuilder::new(0, OpFrame::OpDefine(vec![]))
                    .push(OpFrame::AccountId(acc.id))
                    .push(OpFrame::OpDestroy),
            )
            .build()
            .unwrap();
        acc.balance = get_asset("4.00000 TEST");
        (
            minter.create_account(acc.clone(), "2.00000 TEST", true),
            key,
        )
    };

    // (2) Test destroying the account should fail due to sending funds to self
    let tx = {
        let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
            base: create_tx_header("1.00000 TEST"),
            from: to_acc.id,
            call_fn: 0,
            args: vec![],
            amount: get_asset("0.00000 TEST"),
            memo: vec![],
        }));
        tx.append_sign(&to_acc_key);
        tx
    };

    let res = minter.send_req(rpc::Request::Broadcast(tx));
    match res {
        Some(Err(net::ErrorKind::TxValidation(blockchain::TxErr::ScriptEval(
            script::EvalErr {
                err: script::EvalErrKind::Aborted,
                ..
            },
        )))) => {}
        _ => panic!("Unexpected response: {:?}", res),
    }
}

#[test]
fn fail_destroy_and_send_funds_to_invalid_acc() {
    let minter = TestMinter::new();

    let create_account = |id: AccountId| -> (Account, KeyPair) {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            id,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.script = script::Builder::new()
            .push(
                script::FnBuilder::new(0, OpFrame::OpDefine(vec![script::Arg::AccountId]))
                    .push(OpFrame::OpDestroy),
            )
            .build()
            .unwrap();
        acc.balance = get_asset("4.00000 TEST");
        (
            minter.create_account(acc.clone(), "2.00000 TEST", true),
            key,
        )
    };

    // (1) Create the accounts
    let (acc_1, acc_1_key) = create_account(1);
    let (acc_2, acc_2_key) = create_account(2);

    // (2) Destroy the first account
    {
        let tx = {
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: acc_1.id,
                call_fn: 0,
                args: {
                    let mut args = vec![];
                    args.push_u64(acc_2.id);
                    args
                },
                amount: get_asset("0.00000 TEST"),
                memo: vec![],
            }));
            tx.append_sign(&acc_1_key);
            tx
        };
        let res = minter.send_req(rpc::Request::Broadcast(tx));
        assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
    }

    // (3) Attempt to send funds from destroyed account to another destroyed account
    {
        let des_acc = minter.minter().get_account_info(acc_1.id).unwrap().account;
        assert!(des_acc.destroyed);

        let tx = {
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: acc_2.id,
                call_fn: 0,
                args: {
                    let mut args = vec![];
                    // Send funds to destroyed account
                    args.push_u64(des_acc.id);
                    args
                },
                amount: get_asset("0.00000 TEST"),
                memo: vec![],
            }));
            tx.append_sign(&acc_2_key);
            tx
        };

        let res = minter.send_req(rpc::Request::Broadcast(tx));
        match res {
            Some(Err(net::ErrorKind::TxValidation(blockchain::TxErr::ScriptEval(
                script::EvalErr {
                    err: script::EvalErrKind::AccountNotFound,
                    ..
                },
            )))) => {}
            _ => panic!("Unexpected response: {:?}", res),
        }
    }

    // (4) Attempt to send funds from a destroyed account to an unknown account
    {
        let des_acc = minter.minter().get_account_info(acc_1.id).unwrap().account;
        assert!(des_acc.destroyed);

        let tx = {
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: acc_2.id,
                call_fn: 0,
                args: {
                    let mut args = vec![];
                    // Send funds to unknown account
                    args.push_u64(0xFFFFFFFF);
                    args
                },
                amount: get_asset("0.00000 TEST"),
                memo: vec![],
            }));
            tx.append_sign(&acc_2_key);
            tx
        };

        let res = minter.send_req(rpc::Request::Broadcast(tx));
        match res {
            Some(Err(net::ErrorKind::TxValidation(blockchain::TxErr::ScriptEval(
                script::EvalErr {
                    err: script::EvalErrKind::AccountNotFound,
                    ..
                },
            )))) => {}
            _ => panic!("Unexpected response: {:?}", res),
        }
    }
}

#[test]
fn destroyed_acc_is_indexed_correctly() {
    let minter = TestMinter::new();

    let create_account = |id: AccountId| -> (Account, KeyPair) {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            id,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.script = script::Builder::new()
            .push(
                script::FnBuilder::new(
                    0,
                    OpFrame::OpDefine(vec![script::Arg::AccountId, script::Arg::AccountId]),
                )
                .push(OpFrame::Asset(get_asset("2.00000 TEST")))
                .push(OpFrame::OpTransfer)
                .push(OpFrame::OpDestroy),
            )
            .build()
            .unwrap();
        acc.balance = get_asset("4.00000 TEST");
        (
            minter.create_account(acc.clone(), "2.00000 TEST", true),
            key,
        )
    };

    // (1) Create the accounts
    let (acc_1, acc_1_key) = create_account(1);
    let (acc_2, _) = create_account(2);
    let (acc_3, _) = create_account(3);

    let start_bal = get_asset("4.00000 TEST");

    // (2) Destroy the account
    {
        let tx = {
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: acc_1.id,
                call_fn: 0,
                args: {
                    let mut args = vec![];
                    args.push_u64(acc_2.id);
                    args.push_u64(acc_3.id);
                    args
                },
                amount: get_asset("2.00000 TEST"),
                memo: vec![],
            }));
            tx.append_sign(&acc_1_key);
            tx
        };
        let res = minter.send_req(rpc::Request::Broadcast(tx));
        assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
    }

    // (3) Test indexing in same block
    {
        // Funds being drained and account destroyed information are immediately applied
        let acc_1 = minter.minter().get_account_info(acc_1.id).unwrap().account;
        assert!(acc_1.destroyed);
        assert_eq!(acc_1.balance, Asset::new(0));

        // We can't see the funds from the destroyed account until a block is produced!
        let acc_2 = minter.minter().get_account_info(acc_2.id).unwrap().account;
        assert_eq!(start_bal, acc_2.balance);

        // We transferred some funds before destroying the account, they shouuld be seen immediately
        let acc_3 = minter.minter().get_account_info(acc_3.id).unwrap().account;
        assert_eq!(
            acc_3.balance,
            start_bal.checked_add(get_asset("2.00000 TEST")).unwrap()
        );
    }

    // (4) Test indexing after a block is produced
    {
        minter.produce_block().unwrap();

        // Funds being drained and account destroyed information are immediately applied
        let acc_1 = minter.minter().get_account_info(acc_1.id).unwrap().account;
        assert!(acc_1.destroyed);
        assert_eq!(acc_1.balance, Asset::new(0));

        // We can now see the funds from the destroyed account and use them.
        let acc_2 = minter.minter().get_account_info(acc_2.id).unwrap().account;
        // Add 1 token here since the starting bal is 4, the transaction uses 1 token for the fee
        // and 2 tokens to transfer to account 3.
        assert_eq!(
            start_bal.checked_add(get_asset("1.00000 TEST")).unwrap(),
            acc_2.balance
        );

        // We transferred some funds before destroying the account, they shouuld be seen immediately
        let acc_3 = minter.minter().get_account_info(acc_3.id).unwrap().account;
        assert_eq!(
            acc_3.balance,
            start_bal.checked_add(get_asset("2.00000 TEST")).unwrap()
        );
    }
}

#[test]
fn destroyed_acc_unused_funds_goes_to_correct_acc() {
    let minter = TestMinter::new();

    let create_account = |id: AccountId| -> (Account, KeyPair) {
        let key = KeyPair::gen();
        let mut acc = Account::create_default(
            id,
            Permissions {
                threshold: 1,
                keys: vec![key.0.clone()],
            },
        );
        acc.script = script::Builder::new()
            .push(
                script::FnBuilder::new(
                    0,
                    OpFrame::OpDefine(vec![script::Arg::AccountId, script::Arg::AccountId]),
                )
                .push(OpFrame::Asset(get_asset("1.50000 TEST")))
                .push(OpFrame::OpTransfer)
                .push(OpFrame::OpDestroy),
            )
            .build()
            .unwrap();
        acc.balance = get_asset("4.00000 TEST");
        (
            minter.create_account(acc.clone(), "2.00000 TEST", true),
            key,
        )
    };

    // (1) Create the accounts
    let (acc_1, acc_1_key) = create_account(1);
    let (acc_2, _) = create_account(2);
    let (acc_3, _) = create_account(3);

    // (2) Destroy the account
    {
        let tx = {
            let mut tx = TxVariant::V0(TxVariantV0::TransferTx(TransferTx {
                base: create_tx_header("1.00000 TEST"),
                from: acc_1.id,
                call_fn: 0,
                args: {
                    let mut args = vec![];
                    args.push_u64(acc_2.id);
                    args.push_u64(acc_3.id);
                    args
                },
                amount: get_asset("2.00000 TEST"),
                memo: vec![],
            }));
            tx.append_sign(&acc_1_key);
            tx
        };
        let res = minter.send_req(rpc::Request::Broadcast(tx));
        assert_eq!(res, Some(Ok(rpc::Response::Broadcast)));
    }

    // (3) Test to ensure the log is what we expect
    {
        minter.produce_block().unwrap();

        let block = minter.chain().get_chain_head();
        let receipts = block.receipts();
        assert_eq!(receipts.len(), 1);

        let log = &receipts[0].log;
        let expected_log = vec![
            LogEntry::Transfer(acc_3.id, get_asset("1.50000 TEST")),
            LogEntry::Destroy(acc_2.id),
            LogEntry::Transfer(acc_2.id, get_asset("0.50000 TEST")),
        ];
        assert_eq!(log, &expected_log);
    }
}
