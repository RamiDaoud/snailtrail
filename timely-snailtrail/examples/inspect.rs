use timely_adapter::{
    connect::{make_replayers, open_sockets},
    make_log_records,
};
use timely_snailtrail::{pag, Config};

use timely::dataflow::{
    operators::{capture::replay::Replay, probe::Probe},
    ProbeHandle,
};

use logformat::pair::Pair;

fn main() {
    let workers = std::env::args().nth(1).unwrap().parse::<String>().unwrap();
    let source_peers = std::env::args().nth(2).unwrap().parse::<usize>().unwrap();
    let from_file = if let Some(_) = std::env::args().nth(3) {
        true
    } else {
        false
    };
    let config = Config {
        timely_args: vec!["-w".to_string(), workers],
        source_peers,
        from_file,
    };

    inspector(config);
}

fn inspector(config: Config) {
    // creates one socket per worker in the computation we're examining
    let sockets = if !config.from_file {
        Some(open_sockets(config.source_peers))
    } else {
        None
    };

    timely::execute_from_args(config.timely_args.clone().into_iter(), move |worker| {
        let timer = std::time::Instant::now();

        let index = worker.index();
        if index == 0 {
            println!("{:?}", &config);
        }

        // read replayers from file (offline) or TCP stream (online)
        let replayers = make_replayers(
            worker.index(),
            worker.peers(),
            config.source_peers,
            sockets.clone(),
        );
        let probe = worker.dataflow(|scope| {
            // current dataset (overall times, adding steps in):
            // 2w, debug
            // read_in: ~2500ms
            // log_records no peel: ~2600ms
            // log_records with peel: ~3600ms
            // pag local edges: ~9400ms
            // pag control edges: ~9400ms
            use differential_dataflow::operators::reduce::Count;
            // pag::create_pag(scope, replayers)
                // replayers.replay_into(scope)
                make_log_records(scope, replayers, index)
                // .inspect(|x| println!("{:?}", x))
                // .inspect_batch(|t, x| println!("{:?} ----- {:?}", t, x))
                // .inspect(|x| println!("{:?}", x))
                // .map(|_| 0)
                // .count()
                // .inspect_batch(|t, x| println!("{:?}, count: {:?}", t, x))
                .probe()
        });

        let mut curr_frontier = vec![];
        // while probe.less_equal(&Pair::new(3, std::time::Duration::from_secs(100000000000))) {
        while !probe.done() {
            probe.with_frontier(|f| {
                let f = f.to_vec();
                if f != curr_frontier {
                    println!("w{} frontier: {:?}", index, f);
                    curr_frontier = f;
                }
            });
            worker.step();
        }

        println!("done with stepping.");
        println!("w{} done: {}ms", index, timer.elapsed().as_millis());
    })
    .unwrap();
}
