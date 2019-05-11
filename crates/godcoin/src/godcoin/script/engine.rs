use sodiumoxide::crypto::sign;
use std::borrow::Cow;

use super::{stack::*, *};
use crate::{crypto::PublicKey, tx::TxVariant};

macro_rules! map_err_type {
    ($self:expr, $var:expr) => {
        $var.map_err(|e| $self.new_err(e))
    };
}

pub struct ScriptEngine<'a> {
    script: Cow<'a, Script>,
    tx: Cow<'a, TxVariant>,
    pos: usize,
    stack: Stack,
    sig_pair_pos: usize,
}

impl<'a> ScriptEngine<'a> {
    /// Initializes the scripting engine with a transaction and script.
    ///
    /// Returns `None` if the script is too large.
    pub fn checked_new<T, S>(tx: T, script: S) -> Option<Self>
    where
        T: Into<Cow<'a, TxVariant>>,
        S: Into<Cow<'a, Script>>,
    {
        let script = script.into();
        let tx = tx.into();
        if script.len() > MAX_BYTE_SIZE {
            return None;
        }
        Some(Self {
            script,
            tx,
            pos: 0,
            stack: Stack::new(),
            sig_pair_pos: 0,
        })
    }

    pub fn eval(&mut self) -> Result<bool, EvalErr> {
        macro_rules! pop_multisig_keys {
            ($self:expr, $key_count:expr) => {{
                let mut vec = Vec::with_capacity(usize::from($key_count));
                for _ in 0..$key_count {
                    vec.push(map_err_type!($self, $self.stack.pop_pubkey())?);
                }
                vec
            }};
        }

        self.pos = 0;
        let mut if_marker = 0;
        let mut ignore_else = false;
        while let Some(op) = self.consume_op()? {
            match op {
                // Stack manipulation
                OpFrame::OpNot => {
                    let b = map_err_type!(self, self.stack.pop_bool())?;
                    map_err_type!(self, self.stack.push(!b))?;
                }
                // Control
                OpFrame::OpIf => {
                    if_marker += 1;
                    ignore_else = map_err_type!(self, self.stack.pop_bool())?;
                    if ignore_else {
                        continue;
                    }
                    let req_if_marker = if_marker;
                    self.consume_op_until(|op| {
                        if op == OpFrame::OpIf {
                            if_marker += 1;
                            false
                        } else if op == OpFrame::OpElse {
                            if_marker == req_if_marker
                        } else if op == OpFrame::OpEndIf {
                            let do_break = if_marker == req_if_marker;
                            if_marker -= 1;
                            do_break
                        } else {
                            false
                        }
                    })?;
                }
                OpFrame::OpElse => {
                    if !ignore_else {
                        continue;
                    }
                    let req_if_marker = if_marker;
                    self.consume_op_until(|op| {
                        if op == OpFrame::OpIf {
                            if_marker += 1;
                            false
                        } else if op == OpFrame::OpElse {
                            if_marker == req_if_marker
                        } else if op == OpFrame::OpEndIf {
                            let do_break = if_marker == req_if_marker;
                            if_marker -= 1;
                            do_break
                        } else {
                            false
                        }
                    })?;
                }
                OpFrame::OpEndIf => {
                    if_marker -= 1;
                }
                OpFrame::OpReturn => {
                    if_marker = 0;
                    break;
                }
                // Crypto
                OpFrame::OpCheckSig => {
                    let key = map_err_type!(self, self.stack.pop_pubkey())?;
                    let success = self.check_sigs(1, &[key]);
                    map_err_type!(self, self.stack.push(success))?;
                }
                OpFrame::OpCheckSigFastFail => {
                    let key = map_err_type!(self, self.stack.pop_pubkey())?;
                    if !self.check_sigs(1, &[key]) {
                        return Ok(false);
                    }
                }
                OpFrame::OpCheckMultiSig(threshold, key_count) => {
                    let keys = pop_multisig_keys!(self, key_count);
                    let success = self.check_sigs(usize::from(threshold), &keys);
                    map_err_type!(self, self.stack.push(success))?;
                }
                OpFrame::OpCheckMultiSigFastFail(threshold, key_count) => {
                    let keys = pop_multisig_keys!(self, key_count);
                    if !self.check_sigs(usize::from(threshold), &keys) {
                        return Ok(false);
                    }
                }
                // Handle push ops
                _ => {
                    map_err_type!(self, self.stack.push(op))?;
                }
            }
        }

        if if_marker > 0 {
            return Err(self.new_err(EvalErrType::UnexpectedEOF));
        }

        // Scripts must return true or false
        map_err_type!(self, self.stack.pop_bool())
    }

    fn consume_op_until<F>(&mut self, mut matcher: F) -> Result<(), EvalErr>
    where
        F: FnMut(OpFrame) -> bool,
    {
        loop {
            match self.consume_op()? {
                Some(op) => {
                    if matcher(op) {
                        break;
                    }
                }
                None => return Err(self.new_err(EvalErrType::UnexpectedEOF)),
            }
        }

        Ok(())
    }

    fn consume_op(&mut self) -> Result<Option<OpFrame>, EvalErr> {
        macro_rules! read_bytes {
            ($self:expr, $len:expr) => {
                match $self.script.get($self.pos..$self.pos + $len) {
                    Some(b) => {
                        $self.pos += $len;
                        b
                    }
                    None => {
                        return Err($self.new_err(EvalErrType::UnexpectedEOF));
                    }
                }
            };
            ($self:expr) => {
                match $self.script.get($self.pos) {
                    Some(b) => {
                        $self.pos += 1;
                        *b
                    }
                    None => {
                        return Err($self.new_err(EvalErrType::UnexpectedEOF));
                    }
                }
            };
        }

        if self.pos == self.script.len() {
            return Ok(None);
        }
        let byte = self.script[self.pos];
        self.pos += 1;

        match byte {
            // Push value
            o if o == Operand::PushFalse as u8 => Ok(Some(OpFrame::False)),
            o if o == Operand::PushTrue as u8 => Ok(Some(OpFrame::True)),
            o if o == Operand::PushPubKey as u8 => {
                let slice = read_bytes!(self, sign::PUBLICKEYBYTES);
                let key = PublicKey::from_slice(slice).unwrap();
                Ok(Some(OpFrame::PubKey(key)))
            }
            // Stack manipulation
            o if o == Operand::OpNot as u8 => Ok(Some(OpFrame::OpNot)),
            // Control
            o if o == Operand::OpIf as u8 => Ok(Some(OpFrame::OpIf)),
            o if o == Operand::OpElse as u8 => Ok(Some(OpFrame::OpElse)),
            o if o == Operand::OpEndIf as u8 => Ok(Some(OpFrame::OpEndIf)),
            o if o == Operand::OpReturn as u8 => Ok(Some(OpFrame::OpReturn)),
            // Crypto
            o if o == Operand::OpCheckSig as u8 => Ok(Some(OpFrame::OpCheckSig)),
            o if o == Operand::OpCheckSigFastFail as u8 => Ok(Some(OpFrame::OpCheckSigFastFail)),
            o if o == Operand::OpCheckMultiSig as u8 => {
                let threshold = read_bytes!(self);
                let key_count = read_bytes!(self);
                Ok(Some(OpFrame::OpCheckMultiSig(threshold, key_count)))
            }
            o if o == Operand::OpCheckMultiSigFastFail as u8 => {
                let threshold = read_bytes!(self);
                let key_count = read_bytes!(self);
                Ok(Some(OpFrame::OpCheckMultiSigFastFail(threshold, key_count)))
            }
            _ => Err(self.new_err(EvalErrType::UnknownOp)),
        }
    }

    fn check_sigs(&mut self, threshold: usize, keys: &[PublicKey]) -> bool {
        if threshold == 0 {
            return true;
        } else if threshold > keys.len() || self.sig_pair_pos >= self.tx.signature_pairs.len() {
            return false;
        }

        let mut buf = Vec::with_capacity(4096);
        self.tx.encode(&mut buf);

        let mut valid_threshold = 0;
        let mut key_iter = keys.iter();
        'pair_loop: for pair in &self.tx.signature_pairs[self.sig_pair_pos..] {
            self.sig_pair_pos += 1;
            while let Some(key) = key_iter.next() {
                if key == &pair.pub_key {
                    if key.verify(&buf, &pair.signature) {
                        valid_threshold += 1;
                        if valid_threshold >= threshold {
                            return true;
                        }
                        continue 'pair_loop;
                    } else {
                        break 'pair_loop;
                    }
                }
            }
        }

        false
    }

    fn new_err(&self, err: EvalErrType) -> EvalErr {
        EvalErr::new(self.pos as u32, err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{KeyPair, SigPair, Signature};
    use crate::tx::{SignTx, TransferTx, Tx, TxType};

    #[test]
    fn true_only_script() {
        let mut engine = new_engine(Builder::new().push(OpFrame::True));
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn false_only_script() {
        let mut engine = new_engine(Builder::new().push(OpFrame::False));
        assert!(!engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn if_script() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                .push(OpFrame::OpEndIf),
        );
        assert!(!engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn if_script_with_ret() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::True),
        );
        assert!(!engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::False)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                .push(OpFrame::OpElse)
                    .push(OpFrame::True)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn branch_if() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                .push(OpFrame::OpElse)
                    .push(OpFrame::False)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::False)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                .push(OpFrame::OpElse)
                    .push(OpFrame::True)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn nested_branch_if() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::True)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpElse)
                    .push(OpFrame::False)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::False)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::False)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::False)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpElse)
                    .push(OpFrame::True)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::True)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn fail_invalid_stack_on_return() {
        let key = KeyPair::gen_keypair().0;
        let mut engine = new_engine(Builder::new().push(OpFrame::PubKey(key)));
        assert_eq!(
            engine.eval().unwrap_err().err,
            EvalErrType::InvalidItemOnStack
        );
    }

    #[test]
    fn fail_invalid_if_cmp() {
        let key = KeyPair::gen_keypair().0;
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::PubKey(key))
                .push(OpFrame::OpIf),
        );
        assert_eq!(
            engine.eval().unwrap_err().err,
            EvalErrType::InvalidItemOnStack
        );
    }

    #[test]
    fn fail_unended_if() {
        let mut engine = new_engine(Builder::new().push(OpFrame::True).push(OpFrame::OpIf));
        assert_eq!(engine.eval().unwrap_err().err, EvalErrType::UnexpectedEOF);

        let mut engine = new_engine(Builder::new().push(OpFrame::False).push(OpFrame::OpIf));
        assert_eq!(engine.eval().unwrap_err().err, EvalErrType::UnexpectedEOF);
    }

    #[test]
    fn checksig() {
        let key = KeyPair::gen_keypair();
        let mut engine = new_engine_with_signers(
            &[key.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key.0.clone()))
                .push(OpFrame::OpCheckSig),
        );
        assert!(engine.eval().unwrap());

        let other = KeyPair::gen_keypair();
        let mut engine = new_engine_with_signers(
            &[key.clone()],
            Builder::new()
                .push(OpFrame::PubKey(other.0.clone()))
                .push(OpFrame::OpCheckSig),
        );
        assert!(!engine.eval().unwrap());

        let mut engine = new_engine_with_signers(
            &[other],
            Builder::new()
                .push(OpFrame::PubKey(key.0))
                .push(OpFrame::OpCheckSig),
        );
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checkmultisig_equal_threshold() {
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        let mut engine = new_engine_with_signers(
            &[key_3.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3)),
        );
        assert!(engine.eval().unwrap());
    }

    #[test]
    fn checkmultisig_threshold_unmet() {
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        let mut engine = new_engine_with_signers(
            &[key_3.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(3, 3)),
        );
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checkmultisig_invalid_sig() {
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        let mut engine = new_engine_with_signers(
            &[key_2.clone(), key_1.clone(), KeyPair::gen_keypair()],
            Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3)),
        );
        // This should evaluate to true as the threshold is met, any other invalid signatures are
        // no longer relevant. There is no incentive to inject fake signatures unless the
        // broadcaster wants to pay more in fees.
        assert!(engine.eval().unwrap());

        let mut engine = {
            let to = KeyPair::gen_keypair();
            let script = Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
                .build();

            let mut tx = TransferTx {
                base: Tx {
                    tx_type: TxType::TRANSFER,
                    timestamp: 1500000000,
                    fee: "1 GOLD".parse().unwrap(),
                    signature_pairs: vec![SigPair {
                        // Test valid key with invalid signature
                        pub_key: key_2.0.clone(),
                        signature: Signature(sign::Signature([0; sign::SIGNATUREBYTES])),
                    }],
                },
                from: key_1.clone().0.into(),
                to: to.clone().0.into(),
                amount: "10 GOLD".parse().unwrap(),
                script: script.clone(),
                memo: vec![],
            };
            tx.append_sign(&key_1);

            ScriptEngine::checked_new(TxVariant::TransferTx(tx), script).unwrap()
        };
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checksig_and_checkmultisig_with_if() {
        let key_0 = KeyPair::gen_keypair();
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        // Test threshold is met and tx is signed with key_0
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_2.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpIf)
                    .push(OpFrame::PubKey(key_1.0.clone()))
                    .push(OpFrame::PubKey(key_2.0.clone()))
                    .push(OpFrame::PubKey(key_3.0.clone()))
                    .push(OpFrame::OpCheckMultiSig(2, 3))
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(engine.eval().unwrap());

        // Test tx must be signed with key_0 but threshold is met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_1.clone(), key_2.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpIf)
                    .push(OpFrame::PubKey(key_1.0.clone()))
                    .push(OpFrame::PubKey(key_2.0.clone()))
                    .push(OpFrame::PubKey(key_3.0.clone()))
                    .push(OpFrame::OpCheckMultiSig(2, 3))
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(!engine.eval().unwrap());

        // Test multisig threshold not met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpIf)
                    .push(OpFrame::PubKey(key_1.0.clone()))
                    .push(OpFrame::PubKey(key_2.0.clone()))
                    .push(OpFrame::PubKey(key_3.0.clone()))
                    .push(OpFrame::OpCheckMultiSig(2, 3))
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checksig_and_checkmultisig_with_if_not() {
        let key_0 = KeyPair::gen_keypair();
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        // Test threshold is met and tx is signed with key_0
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_2.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpNot)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
        );
        assert!(engine.eval().unwrap());

        // Test tx must be signed with key_0 but threshold is met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_1.clone(), key_2.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpNot)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
        );
        assert!(!engine.eval().unwrap());

        // Test multisig threshold not met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpNot)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
        );
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checksig_and_checkmultisig_with_fast_fail() {
        let key_0 = KeyPair::gen_keypair();
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        // Test tx must be signed with key_0 but threshold is met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_1.clone(), key_2.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSigFastFail)
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
        );
        assert!(!engine.eval().unwrap());

        // Test multisig threshold not met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSigFastFail(2, 3))
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
        );
        assert!(!engine.eval().unwrap());
    }

    fn new_engine<'a>(builder: Builder) -> ScriptEngine<'a> {
        let from = KeyPair::gen_keypair();
        new_engine_with_signers(&[from], builder)
    }

    fn new_engine_with_signers<'a>(keys: &[KeyPair], b: Builder) -> ScriptEngine<'a> {
        let to = KeyPair::gen_keypair();
        let script = b.build();

        let mut tx = TransferTx {
            base: Tx {
                tx_type: TxType::TRANSFER,
                timestamp: 1500000000,
                fee: "1 GOLD".parse().unwrap(),
                signature_pairs: vec![],
            },
            from: keys[0].clone().0.into(),
            to: to.clone().0.into(),
            amount: "10 GOLD".parse().unwrap(),
            script: script.clone(),
            memo: vec![],
        };
        for key in keys {
            tx.append_sign(&key);
        }

        ScriptEngine::checked_new(TxVariant::TransferTx(tx), script).unwrap()
    }
}
