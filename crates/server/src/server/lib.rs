use actix::prelude::*;
use actix_web::{middleware, web, App, HttpResponse, HttpServer};
use futures::{future::ok, Future};
use godcoin::{net::*, prelude::*};
use log::{error, info};
use std::{io::Cursor, path::PathBuf, sync::Arc};

pub mod minter;
pub mod net;

pub mod prelude {
    pub use super::minter::*;
    pub use super::net::*;
}

use prelude::*;

pub struct ServerConfig {
    pub home: PathBuf,
    pub minter_key: KeyPair,
    pub bind_addr: String,
}

#[derive(Clone)]
pub struct ServerData {
    pub chain: Arc<Blockchain>,
    pub minter: Addr<Minter>,
}

pub fn start(config: ServerConfig) {
    let blockchain = Arc::new(Blockchain::new(&config.home));
    info!(
        "Using height in block log at {}",
        blockchain.get_chain_height()
    );

    if blockchain.get_block(0).is_none() {
        let info = blockchain.create_genesis_block(config.minter_key.clone());
        info!("=> Generated new block chain");
        info!("=> {:?}", info.script);
        for (index, key) in info.wallet_keys.iter().enumerate() {
            info!("=> Wallet key {}: {}", index + 1, key.1.to_wif());
        }
    }

    let wallet_addr = blockchain.get_owner().wallet;
    let minter = Minter::new(Arc::clone(&blockchain), config.minter_key, wallet_addr).start();
    minter.do_send(minter::StartProductionLoop);

    HttpServer::new(move || {
        App::new()
            .data(ServerData {
                chain: Arc::clone(&blockchain),
                minter: minter.clone(),
            })
            .wrap(middleware::Logger::new(r#"%a "%r" %s %T"#))
            .service(
                web::resource("/")
                    .data({
                        // Limit 64 KiB
                        web::PayloadConfig::default().limit(65536)
                    })
                    .route(web::post().to_async(index)),
            )
    })
    .bind(config.bind_addr)
    .unwrap()
    .start();
}

fn index(
    data: web::Data<ServerData>,
    body: bytes::Bytes,
) -> Box<Future<Item = HttpResponse, Error = ()>> {
    match MsgRequest::deserialize(&mut Cursor::new(&body)) {
        Ok(msg_req) => Box::new(handle_request(&data, msg_req).map(IntoHttpResponse::into_res)),
        Err(e) => match e.kind() {
            _ => {
                error!("Unknown error occurred during deserialization: {:?}", e);
                Box::new(ok(MsgResponse::Error(ErrorKind::Io).into_res()))
            }
        },
    }
}

pub fn handle_request(
    data: &ServerData,
    req: MsgRequest,
) -> Box<Future<Item = MsgResponse, Error = ()> + Send> {
    match req {
        MsgRequest::GetProperties => {
            let props = data.chain.get_properties();
            Box::new(ok(MsgResponse::GetProperties(props)))
        }
        MsgRequest::GetBlock(height) => match data.chain.get_block(height) {
            Some(block) => Box::new(ok(MsgResponse::GetBlock(block.as_ref().clone()))),
            None => Box::new(ok(MsgResponse::Error(ErrorKind::InvalidHeight))),
        },
        MsgRequest::Broadcast(tx) => {
            let fut = data.minter.send(minter::PushTx(tx)).then(|res| {
                Ok(match res.unwrap() {
                    Ok(_) => MsgResponse::Broadcast(),
                    Err(e) => MsgResponse::Error(ErrorKind::TxValidation(e)),
                })
            });
            Box::new(fut)
        }
    }
}
