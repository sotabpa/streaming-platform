use std::{collections::HashMap, fmt::Debug};
use log::*;
use cookie::Cookie;
use ws::{Request, Builder, Handler, Sender, Message, Handshake, CloseCode};
use sp_dto::bytes::{Buf, BufMut};
use sp_dto::uuid::Uuid;
use sp_dto::{MsgMeta, MsgKind, MsgSource};
use crate::AuthData;
use crate::proto::{ClientKind, ServerMsg, ClientMsg, MagicBall, MagicBall2};
use crate::error::Error;

struct WsServer {
    net_addr: Option<String>,
    auth_data: Option<AuthData>,
    ws: Sender,
    config: HashMap<String, String>,
    tx: crossbeam::channel::Sender<ServerMsg>,
    client_kind: Option<ClientKind>,
    addr: Option<String>,
    link_magic_ball: Option<MagicBall2>
}

impl Handler for WsServer {

    fn on_open(&mut self, hs: Handshake) -> ws::Result<()> {

        debug!("got client {}", self.ws.connection_id());

        match hs.remote_addr()? {
            Some(net_addr) => {
                self.net_addr = Some(net_addr.clone());
                debug!("Connection with {} now open", net_addr);
            }
            None => debug!("No remote addr present.")
        }

        match hs.request.header("Cookie") {
            Some(cookie) => {
                let cookie = std::str::from_utf8(cookie)?;

                debug!("Cookie: {}", cookie);

                match Cookie::parse(cookie) {
                    Ok(cookie) => {
                        debug!("Cookie: {:?}", cookie.name_value());
                        match cookie.name() {
                            "addr" => {
                                let addr = cookie.value();
                                let addr = Uuid::new_v4().to_string();
                                
                                self.client_kind = Some(ClientKind::App);
                                self.addr = Some(addr.clone());
                                self.tx.send(ServerMsg::AddClient(addr, self.ws.clone()));
                            }
                            _ => debug!("No addr present.")
                        }
                    }
                    Err(err) => error!("Cookie parse error: {}", err)
                }

                return Ok(());
            }
            None => {
                debug!("No Cookie header present.")
            }
        }

        match hs.request.header("Service") {
            Some(addr) => {                                
                let addr = std::str::from_utf8(addr)?;

                debug!("Service: {}", addr);

                self.client_kind = Some(ClientKind::Service);

                self.addr = Some(addr.to_owned());
                self.tx.send(ServerMsg::AddClient(addr.to_owned(), self.ws.clone()));
            }
            None => {
                debug!("No Service header present.")
            }
        }

        match hs.request.header("Hub") {
            Some(addr) => {                                
                let addr = std::str::from_utf8(addr)?;

                debug!("Hub: {}", addr);

                self.client_kind = Some(ClientKind::Hub);

                self.addr = Some(addr.to_owned());
                self.tx.send(ServerMsg::AddClient(addr.to_owned(), self.ws.clone()));
            }
            None => {
                debug!("No Hub header present.")
            }
        }

        /*

        if let Some(cookie) = hs.request.header("Cookie") {
            match Cookie::parse_header(&cookie.to_vec().into()) {
                Ok(cookie_header) => {
                    self.auth_data = get_auth_data(Some(&cookie_header));
                    match self.auth_data {
                        None => {
                            debug!("ws auth attempt failed, sending close.");
                            //self.ws.close(CloseCode::Normal);
                        }
                        _ => {}
                    }
                }
                Err(e) => error!("ws cookie parse error. {}", e)
            }
        }
        */                

        Ok(())
    }

    fn on_message(&mut self, msg: Message) -> ws::Result<()> {

        debug!("got message");

        match &self.addr {
            Some(addr) => {
                match &self.client_kind {
                    Some(client_kind) => {
                        match msg {
                            Message::Text(data) => {},
                            Message::Binary(mut data) => {

                                let (res, len) = {
                                    let mut buf = std::io::Cursor::new(&data);
                                    let len = buf.get_u32_be() as usize;

                                    match len > data.len() - 4 {
                                        true => {
                                            let custom_error = std::io::Error::new(std::io::ErrorKind::Other, "oh no!");
                                            return Err(ws::Error::new(ws::ErrorKind::Io(custom_error), ""));
                                        }
                                        false => (serde_json::from_slice::<MsgMeta>(&data[4..len + 4]), len)
                                    }
                                };

                                match res {
                                    Ok(mut msg_meta) => {
                                        debug!("Sending message: {:#?}", msg_meta);

                                        match client_kind {
                                            ClientKind::App => {
                                                match &self.link_magic_ball {
                                                    Some(magic_ball) => {                                                        
                                                        msg_meta.tx = "AppHub".to_owned();

                                                        match msg_meta.source {
                                                            MsgSource::Component(ref mut spec) => {
                                                                spec.client_addr = addr.clone();
                                                            }
                                                            _ => {}
                                                        }

                                                        match serde_json::to_vec(&msg_meta) {
                                                            Ok(mut msg_meta) => {                                                                
                                                                let mut payload_with_attachments: Vec<_> = data.drain(4 + len..).collect();
                                                                let mut buf = vec![];

                                                                buf.put_u32_be(msg_meta.len() as u32);

                                                                buf.append(&mut msg_meta);
                                                                buf.append(&mut payload_with_attachments);

                                                                magic_ball.send_data(buf);
                                                            }
                                                            Err(err) => {
                                                                error!("MsgMeta serialization failed!")
                                                            }
                                                        }                                                        
                                                    }
                                                    None => {
                                                        error!("Magic ball missing!");
                                                    }
                                                }
                                            }
                                            ClientKind::Service | ClientKind::Hub => {
                                                self.tx.send(ServerMsg::SendMsg(msg_meta.rx, data));
                                            }
                                        }                                                                                                    
                                    }
                                    Err(err) => {
                                        error!("MsgMeta deserialization failed!")
                                    }
                                }                                
                            }
                        }                        
                    }
                    None => {
                        error!("Missing client_kind for {}", addr);
                    }
                }                
            }
            None => {
                debug!("Client is unauthorized.");
            }
        }

        Ok(())
    }

    fn on_close(&mut self, code: CloseCode, reason: &str) {

        debug!("closed");

        match code {

            CloseCode::Normal => {}//debug!("The client is done with the connection."),

            CloseCode::Away   => {}//debug!("The client is leaving the site."),

            _ => {}//debug!("The client encountered an error: {}", reason),

        }

    }

    fn on_error(&mut self, err: ws::Error) {
        //debug!("The server encountered an error: {:?}", err);
    }

}

pub fn start(host: String, port: u16, config: HashMap<String, String>) {

    let (tx, rx) = crossbeam::channel::unbounded();

    let mut server = Builder::new().build(|ws| {

        WsServer {
            net_addr: None,
            auth_data: None,
            ws,
            config: config.clone(),
            tx: tx.clone(),
            client_kind: None,
            addr: None,
            link_magic_ball: None
        }

    }).unwrap();

    let clients = std::thread::Builder::new()
        .name("clients".to_owned())
        .spawn(move || {
            let mut clients = HashMap::new();            

            loop {
                let msg = rx.recv().unwrap();

                match msg {
                    ServerMsg::AddClient(addr, sender) => {
                        debug!("Adding client {}", &addr);
                        clients.insert(addr, sender);                                
                    }
                    ServerMsg::SendMsg(addr, res) => {
                        match clients.get(&addr) {
                            Some(sender) => {
                                debug!("Sending message to client {}", &addr);
                                sender.send(res);                                
                            }
                            None => {
                                debug!("Client not found: {}", &addr);
                            }
                        }
                    }
                    _ => {}
                }                
            }
        })
        .unwrap();

    server.listen(format!("{}:{}", host, port));
}

pub fn start_with_link(host: String, port: u16, link_client_name: String, link_to_host: String, config: HashMap<String, String>) {

    let (tx, rx) = crossbeam::channel::unbounded();    

    let (handle, magic_ball) = crate::simple::client::connect2(link_client_name, link_to_host, ClientKind::Hub, Some(tx.clone())).unwrap();

    let mut server = Builder::new().build(|ws| {

        WsServer {
            net_addr: None,
            auth_data: None,
            ws,
            config: config.clone(),
            tx: tx.clone(),
            client_kind: None,
            addr: None,
            link_magic_ball: Some(magic_ball.clone())
        }

    }).unwrap();

    let clients = std::thread::Builder::new()
        .name("clients".to_owned())
        .spawn(move || {
            let mut clients = HashMap::new();            

            loop {
                let msg = rx.recv().unwrap();

                match msg {
                    ServerMsg::AddClient(addr, sender) => {
                        debug!("Adding client {}", &addr);
                        clients.insert(addr, sender);                                
                    }
                    ServerMsg::SendMsg(addr, data) => {
                        match clients.get(&addr) {
                            Some(sender) => {
                                debug!("Sending message to client {}", &addr);
                                sender.send(data);                                
                            }
                            None => {
                                debug!("Client not found: {}", &addr);
                            }
                        }
                    }
                    _ => {}
                }                
            }
        })
        .unwrap();

    server.listen(format!("{}:{}", host, port));
}

#[test]
fn test_scenarios() {
    let server = std::thread::Builder::new()
        .name("server".to_owned())
        .spawn(|| {
            start("0.0.0.0".to_owned(), 60000, HashMap::new())
        })
        .unwrap();

    let host = "ws://127.0.0.1:60000";

    let (tx, rx) = crossbeam::channel::unbounded();

    let (handle, sender) = connect("hello".to_owned(), host.to_owned(), tx).unwrap();

    handle.join().unwrap();
}
