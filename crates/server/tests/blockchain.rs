use actix::prelude::*;
use godcoin::{
    constants,
    prelude::{net::ErrorKind, verify::TxErr, *},
};

mod common;
pub use common::*;

#[test]
fn fresh_blockchain() {
    System::run(|| {
        let minter = TestMinter::new();
        let chain = minter.chain();
        assert!(chain.get_block(0).is_some());
        assert!(chain.get_block(1).is_some());
        assert_eq!(chain.get_chain_height(), 1);

        let owner = chain.get_owner();
        assert_eq!(owner.minter, minter.genesis_info().minter_key.0);
        assert_eq!(
            owner.script,
            script::Builder::new().push(OpFrame::False).build()
        );
        assert_eq!(owner.wallet, (&minter.genesis_info().script).into());

        assert!(chain.get_block(2).is_none());
        assert_eq!(chain.index_status(), IndexStatus::Complete);
        System::current().stop();
    })
    .unwrap();
}

#[test]
fn reindexed_blockchain() {
    System::run(|| {
        let minter = TestMinter::new();

        let from_addr = ScriptHash::from(&minter.genesis_info().script);
        let from_bal = minter.chain().get_balance(&from_addr, &[]).unwrap();
        let to_addr = KeyPair::gen();
        let amount = get_asset("1.0000 GRAEL");

        let tx = {
            let mut tx = TransferTx {
                base: create_tx_header(TxType::TRANSFER, "1.0000 GRAEL"),
                from: from_addr.clone(),
                to: (&to_addr.0).into(),
                amount,
                memo: vec![],
                script: minter.genesis_info().script.clone(),
            };
            tx.append_sign(&minter.genesis_info().wallet_keys[3]);
            tx.append_sign(&minter.genesis_info().wallet_keys[0]);
            TxVariant::TransferTx(tx)
        };
        let fut = minter.request(MsgRequest::Broadcast(tx));
        Arbiter::spawn(
            fut.and_then(move |res| {
                assert_eq!(res, MsgResponse::Broadcast());
                minter.produce_block().map(|_| minter)
            })
            .and_then(move |mut minter| {
                minter.unindexed();

                let chain = minter.chain();
                assert_eq!(chain.index_status(), IndexStatus::None);
                assert!(chain.get_block(0).is_none());

                chain.reindex();
                assert_eq!(chain.index_status(), IndexStatus::Complete);
                assert!(chain.get_block(0).is_some());
                assert!(chain.get_block(1).is_some());
                assert!(chain.get_block(2).is_some());
                assert!(chain.get_block(3).is_none());
                assert_eq!(chain.get_chain_height(), 2);

                let owner = chain.get_owner();
                assert_eq!(owner.minter, minter.genesis_info().minter_key.0);
                assert_eq!(
                    owner.script,
                    script::Builder::new().push(OpFrame::False).build()
                );
                assert_eq!(owner.wallet, (&minter.genesis_info().script).into());

                let cur_bal = chain.get_balance(&to_addr.0.into(), &[]);
                assert_eq!(cur_bal, Some(amount));

                // The fee transfers back to the minter wallet in the form of a reward tx so it
                // must not be subtracted during the assertion
                let cur_bal = chain.get_balance(&from_addr, &[]);
                assert_eq!(cur_bal, from_bal.sub(amount));

                System::current().stop();
                Ok(())
            }),
        );
    })
    .unwrap();
}

#[test]
fn tx_dupe() {
    System::run(|| {
        let minter = TestMinter::new();

        let mut tx = MintTx {
            base: create_tx_header(TxType::MINT, "0.0000 GRAEL"),
            to: (&minter.genesis_info().script).into(),
            amount: get_asset("10.0000 GRAEL"),
            attachment: vec![],
            attachment_name: "".to_owned(),
            script: minter.genesis_info().script.clone(),
        };

        tx.append_sign(&minter.genesis_info().wallet_keys[1]);
        tx.append_sign(&minter.genesis_info().wallet_keys[0]);

        let tx = TxVariant::MintTx(tx);
        let fut = minter.request(MsgRequest::Broadcast(tx.clone()));
        Arbiter::spawn(
            fut.and_then(move |res| {
                assert!(!res.is_err(), format!("{:?}", res));

                minter.request(MsgRequest::Broadcast(tx))
            })
            .and_then(|res| {
                assert!(res.is_err());
                assert_eq!(
                    res,
                    MsgResponse::Error(ErrorKind::TxValidation(TxErr::TxDupe))
                );

                System::current().stop();
                Ok(())
            }),
        );
    })
    .unwrap();
}

#[test]
fn tx_expired() {
    use godcoin::constants::TX_EXPIRY_TIME;

    System::run(|| {
        let minter = TestMinter::new();
        let time = util::get_epoch_ms();

        let tx = MintTx {
            base: create_tx_header_with_ts(TxType::MINT, "0.0000 GRAEL", time + TX_EXPIRY_TIME),
            to: (&minter.genesis_info().script).into(),
            amount: get_asset("10.0000 GRAEL"),
            attachment: vec![],
            attachment_name: "".to_owned(),
            script: minter.genesis_info().script.clone(),
        };

        let tx = TxVariant::MintTx(tx);
        let fut = minter.request(MsgRequest::Broadcast(tx));
        Arbiter::spawn(fut.then(move |res| {
            let res = res.unwrap();
            assert!(res.is_err());
            assert_eq!(
                res,
                MsgResponse::Error(ErrorKind::TxValidation(TxErr::TxExpired))
            );

            System::current().stop();
            Ok(())
        }));
    })
    .unwrap();
}

#[test]
fn tx_far_in_the_future() {
    System::run(|| {
        let minter = TestMinter::new();
        let time = util::get_epoch_ms();

        let tx = MintTx {
            base: create_tx_header_with_ts(TxType::MINT, "0.0000 GRAEL", time + 4000),
            to: (&minter.genesis_info().script).into(),
            amount: get_asset("10.0000 GRAEL"),
            attachment: vec![],
            attachment_name: "".to_owned(),
            script: minter.genesis_info().script.clone(),
        };

        let tx = TxVariant::MintTx(tx);
        let fut = minter.request(MsgRequest::Broadcast(tx));
        Arbiter::spawn(fut.then(move |res| {
            let res = res.unwrap();
            assert!(res.is_err());
            assert_eq!(
                res,
                MsgResponse::Error(ErrorKind::TxValidation(TxErr::TxExpired))
            );

            System::current().stop();
            Ok(())
        }));
    })
    .unwrap();
}

#[test]
fn tx_script_too_large_err() {
    System::run(|| {
        let minter = TestMinter::new();

        let tx = MintTx {
            base: create_tx_header(TxType::MINT, "0.0000 GRAEL"),
            to: (&minter.genesis_info().script).into(),
            amount: get_asset("10.0000 GRAEL"),
            attachment: vec![],
            attachment_name: "".to_owned(),
            script: Script::new((0..=constants::MAX_SCRIPT_BYTE_SIZE).map(|_| 0).collect()),
        };

        let tx = TxVariant::MintTx(tx);
        let fut = minter.request(MsgRequest::Broadcast(tx));
        Arbiter::spawn(fut.and_then(move |res| {
            assert!(res.is_err());
            assert_eq!(
                res,
                MsgResponse::Error(ErrorKind::TxValidation(TxErr::TxTooLarge))
            );

            System::current().stop();
            Ok(())
        }));
    })
    .unwrap();
}

#[test]
fn tx_too_many_signatures_err() {
    System::run(|| {
        let minter = TestMinter::new();

        let mut tx = MintTx {
            base: create_tx_header(TxType::MINT, "0.0000 GRAEL"),
            to: (&minter.genesis_info().script).into(),
            amount: get_asset("10.0000 GRAEL"),
            attachment: vec![],
            attachment_name: "".to_owned(),
            script: Script::new(vec![]),
        };
        (0..=constants::MAX_TX_SIGNATURES).for_each(|_| tx.append_sign(&KeyPair::gen()));

        let tx = TxVariant::MintTx(tx);
        let fut = minter.request(MsgRequest::Broadcast(tx));
        Arbiter::spawn(fut.and_then(move |res| {
            assert!(res.is_err());
            assert_eq!(
                res,
                MsgResponse::Error(ErrorKind::TxValidation(TxErr::TooManySignatures))
            );

            System::current().stop();
            Ok(())
        }));
    })
    .unwrap();
}
