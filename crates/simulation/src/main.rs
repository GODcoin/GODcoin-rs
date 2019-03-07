use actix::actors::signal;
use actix::prelude::*;
use bytes::BytesMut;
use godcoin_p2p::{session::*, cmd, Network, Payload};
use log::{error, info};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
    cell::RefCell,
    rc::Rc
};
use tokio::{prelude::*, timer::Delay};

struct Signals;

impl Actor for Signals {
    type Context = Context<Self>;
}

impl Handler<signal::Signal> for Signals {
    type Result = ();

    fn handle(&mut self, msg: signal::Signal, _: &mut Context<Self>) {
        if let signal::SignalType::Int = msg.0 {
            info!("SIGINT received, exiting");
            System::current().stop();
        }
    }
}

struct NetState {
    net_id: usize,
    messages: HashSet<BytesMut>,
    msg_counter: Rc<RefCell<usize>>,
    msg_threshold: usize,
}

impl NetState {
    pub fn new(net_id: usize, msg_counter: Rc<RefCell<usize>>, msg_threshold: usize) -> Self {
        NetState {
            net_id,
            messages: HashSet::new(),
            msg_counter,
            msg_threshold
        }
    }
}

fn connected(state: &mut NetState, ses: SessionInfo) {
    match ses.conn_type {
        ConnectionType::Inbound => {
            info!(
                "[net:{}] Accepted connection -> {}",
                state.net_id, ses.peer_addr
            );
        }
        ConnectionType::Outbound => {
            info!(
                "[net:{}] Connected to node -> {}",
                state.net_id, ses.peer_addr
            );
        }
    }
}

fn disconnected(state: &mut NetState, ses: SessionInfo) {
    info!(
        "[net:{}] Connection disconnected -> {}",
        state.net_id, ses.peer_addr
    );
}

fn message(state: &mut NetState, id: SessionId, payload: &Payload) -> bool {
    info!(
        "[net:{}] Received message from {} with: {:?}",
        state.net_id, id, payload
    );
    let broadcast = state.messages.contains(&payload.id);
    *state.msg_counter.borrow_mut() += 1;
    if *state.msg_counter.borrow() == state.msg_threshold {
        info!("Threshold reached => evicting cached messages");
        state.messages.clear();
    }
    broadcast
}

fn main() {
    let env = env_logger::Env::new()
        .filter_or(env_logger::DEFAULT_FILTER_ENV, "godcoin_simulation=debug");
    env_logger::init_from_env(env);

    let sys = System::new("simulation");

    {
        let sig_addr = Signals.start();
        let addr = signal::ProcessSignals::from_registry();
        addr.do_send(signal::Subscribe(sig_addr.recipient()));
    }

    let nets = {
        let net_count = 3;
        let port = 7777;
        let mut nets = Vec::with_capacity(net_count);
        let msg_counter = Rc::new(RefCell::new(0));
        // The threshold is always one less to exclude the current network from being counted
        let threshold = net_count - 1;
        for net_id in 0..net_count {
            let msg_counter = Rc::clone(&msg_counter);
            let net = Network::new(NetState::new(net_id, msg_counter, threshold), message)
                .on_connect(connected)
                .on_disconnect(disconnected)
                .start();
            net.do_send(cmd::Listen(
                format!("127.0.0.1:{}", port + net_id).parse().unwrap(),
            ));
            info!("[net:{}] Accepting connections on 127.0.0.1:7777", net_id);
            nets.push(net);
        }
        nets
    };

    nets[1].do_send(cmd::Connect("127.0.0.1:7777".parse().unwrap()));
    nets[2].do_send(cmd::Connect("127.0.0.1:7777".parse().unwrap()));
    nets[2].do_send(cmd::Connect("127.0.0.1:7778".parse().unwrap()));

    let deadline = Instant::now() + Duration::from_secs(1);
    Arbiter::spawn(
        Delay::new(deadline)
            .and_then(move |_| {
                let payload = Payload {
                    id: BytesMut::from(vec![1, 2, 3]),
                    msg: BytesMut::from(vec![4, 5, 6]),
                };
                info!("[net:2] Broadcasting message: {:?}", &payload);
                nets[2].do_send(cmd::Broadcast(payload));
                Ok(())
            })
            .map_err(|e| {
                error!("Timer failed: {:?}", e);
            }),
    );

    sys.run();
}