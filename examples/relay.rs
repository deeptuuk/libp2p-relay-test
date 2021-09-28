use std::{
    error::Error,
    fmt,
    str::FromStr,
    task::{Context, Poll},
    time::Duration
};

use structopt::StructOpt;

use futures::executor::block_on;
use futures::stream::StreamExt;

use libp2p::{
    core::upgrade, 
    identity::{self, ed25519}, 
    floodsub::{self, Floodsub, FloodsubEvent},
    NetworkBehaviour, 
    PeerId, 
    Swarm,
    swarm::{NetworkBehaviourEventProcess, SwarmBuilder, SwarmEvent},
    dns::DnsConfig,
    tcp::TcpConfig,
    plaintext,
    Transport,
    yamux::YamuxConfig,
    relay::{Relay, RelayConfig, new_transport_and_behaviour}
};

// Listen on all interfaces and whatever port the OS assigns
const DEFAULT_RELAY_ADDRESS: &str = "/ip4/0.0.0.0/tcp/0";

fn main() -> Result<(), Box<dyn Error>> {

    let opt = Opt::from_args();
    println!("opt: {:?}", opt);

    let local_key: identity::Keypair = generate_ed25519(opt.secret_key_seed);
    let local_peer_id = PeerId::from(local_key.public());
    println!("Local peer id: {:?}", local_peer_id);    

    let transport = block_on(DnsConfig::system(TcpConfig::new()))?;

    let relay_config = RelayConfig {
        connection_idle_timeout: Duration::from_secs(60 * 60),
        ..Default::default()
    };    

    let (relay_wrapped_transport, relay_behaviour) = new_transport_and_behaviour(
        relay_config,
        transport,
    );    

    // Create a Floodsub topic
    let floodsub_topic = floodsub::Topic::new("chat");    

    let mut behaviour = MyBehaviour {
        relay: relay_behaviour,
        floodsub: Floodsub::new(local_peer_id),
    };    

    behaviour.floodsub.subscribe(floodsub_topic.clone());

    let plaintext = plaintext::PlainText2Config {
        local_public_key: local_key.public(),
    };    

    let transport = relay_wrapped_transport
        .upgrade(upgrade::Version::V1)
        .authenticate(plaintext)
        .multiplex(YamuxConfig::default())
        .boxed();    

    let mut swarm = Swarm::new(transport, behaviour, local_peer_id);

    match opt.mode {
        Mode::Relay => {
            let address = get_relay_address(&opt);
            swarm.listen_on(address.parse()?)?;
            println!("starting listening as relay on {}", &address);
        }
        Mode::ClientListen => {
            let relay_address = get_relay_peer_address(&opt);
            swarm.listen_on(relay_address.parse()?)?;
            println!("starting client listener via relay on {}", &relay_address);
        }
        Mode::ClientDial => {
            let client_listen_address = get_client_listen_address(&opt);
            swarm.dial_addr(client_listen_address.parse()?)?;
            println!("starting as client dialer on {}", client_listen_address);
        }
    }   

    block_on(futures::future::poll_fn(move |cx: &mut Context<'_>| {
        loop {
            match swarm.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        print_listener_peer(&address, &opt.mode, local_peer_id)
                    }
                    _ => println!("{:?}", event),
                },
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => break,
            }
        }
        Poll::Pending
    }))    
}

fn print_listener_peer(addr: &libp2p::Multiaddr, mode: &Mode, local_peer_id: PeerId) -> () {
    match mode {
        Mode::Relay => {
            println!(
                "Peer that act as Relay can access on: `{}/p2p/{}/p2p-circuit`",
                addr, local_peer_id
            );
        }
        Mode::ClientListen => {
            println!(
                "Peer that act as Client Listen can access on: `/p2p/{}/{}`",
                addr, local_peer_id
            );
        }
        Mode::ClientDial => {
            println!("Peer that act as Client Dial Listening on {:?}", addr);
        }
    }
}

/// Get the address for relay mode
fn get_relay_address(opt: &Opt) -> String {
    match &opt.address {
        Some(address) => address.clone(),
        None => {
            println!("--address argument was not provided, will use the default listening relay address: {}",DEFAULT_RELAY_ADDRESS);
            DEFAULT_RELAY_ADDRESS.to_string()
        }
    }
}

/// Get the address for client_listen mode
fn get_relay_peer_address(opt: &Opt) -> String {
    match &opt.address {
        Some(address) => address.clone(),
        None => panic!("Please provide relayed listen address such as: <addr-relay-server>/p2p/<peer-id-relay-server>/p2p-circuit"),
    }
}

/// Get the address for client-dial mode
fn get_client_listen_address(opt: &Opt) -> String {
    match &opt.address {
        Some(address) => address.clone(),
        None => panic!("Please provide client listen address such as: <addr-relay-server>/p2p/<peer-id-relay-server>/p2p-circuit/p2p/<peer-id-listening-relay-client>")
    }
}

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
struct MyBehaviour {
    relay: Relay,
    floodsub: Floodsub,
}

impl NetworkBehaviourEventProcess<FloodsubEvent> for MyBehaviour {
    // Called when `floodsub` produces an event.
    fn inject_event(&mut self, message: FloodsubEvent) {
        if let FloodsubEvent::Message(message) = message {
            println!(
                "Received: '{:?}' from {:?}",
                String::from_utf8_lossy(&message.data),
                message.source
            );
        }
    }
}

impl NetworkBehaviourEventProcess<()> for MyBehaviour {
    // Called when `relay` produces an event.
    fn inject_event(&mut self, message: ()) {
        println!("----------This is a test when relay produces an event--------")
    }    
}




// #[derive(Debug)]
// enum Event {
//     Relay(()),
//     Floodsub(FloodsubEvent),
// }

// impl From<()> for Event {
//     fn from(_: ()) -> Self {
//         Event::Relay(())
//     }
// }

// impl From<FloodsubEvent> for Event {
//     fn from(v: FloodsubEvent) -> Self {
//         Self::Floodsub(v)
//     }
// }

fn generate_ed25519(secret_key_seed: u8) -> identity::Keypair {
    let mut bytes = [0u8; 32];
    bytes[0] = secret_key_seed;

    let secret_key = ed25519::SecretKey::from_bytes(&mut bytes)
        .expect("this returns `Err` only if the length is wrong; the length is correct; qed");
    identity::Keypair::Ed25519(secret_key.into())
}


#[derive(Debug, StructOpt)]
enum Mode {
    Relay,
    ClientListen,
    ClientDial,
}

impl FromStr for Mode {
    type Err = ModeError;
    fn from_str(mode: &str) -> Result<Self, Self::Err> {
        match mode {
            "relay" => Ok(Mode::Relay),
            "client-listen" => Ok(Mode::ClientListen),
            "client-dial" => Ok(Mode::ClientDial),
            _ => Err(ModeError {}),
        }
    }
}

#[derive(Debug)]
struct ModeError {}
impl Error for ModeError {}
impl fmt::Display for ModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Could not parse a mode")
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "libp2p relay")]
struct Opt {
    /// The mode (relay, client-listen, client-dial)
    #[structopt(long)]
    mode: Mode,

    /// Fixed value to generate deterministic peer id
    #[structopt(long)]
    secret_key_seed: u8,

    /// The listening address
    #[structopt(long)]
    address: Option<String>,
}