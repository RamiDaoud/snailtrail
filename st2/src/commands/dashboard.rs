use crate::pag;
use crate::pag::PagEdge;
use crate::STError;
use crate::PagData;
use crate::commands::algo::{KHops, KHopsSummary};
use crate::{MetricsData, KHopSummaryData};
use crate::commands::metrics::Metrics;
use crate::InvariantData;
use crate::commands::invariants::Invariants;
use crate::{EpochData, OperatorData, MessageData};

use timely::dataflow::Stream;
use timely::dataflow::operators::inspect::Inspect;

use std::time::Duration;
use std::sync::mpsc;
use std::sync::{Mutex, Arc};
use std::convert::TryInto;

use st2_logformat::pair::Pair;

use tdiag_connect::receive as connect;
use tdiag_connect::receive::ReplaySource;


/// Creates an online dashboard for ST2.
pub fn run(
    timely_configuration: timely::Configuration,
    replay_source: ReplaySource,
    pag_send: Arc<Mutex<mpsc::Sender<(u64, PagData)>>>,
    epoch_max: Option<u64>,
    operator_max: Option<u64>,
    message_max: Option<u64>,
) -> Result<(), STError> {

    timely::execute(timely_configuration, move |worker| {
        let index = worker.index();

        let pag_send1 = pag_send.lock().expect("cannot lock pag_send").clone();
        let pag_send2 = pag_send.lock().expect("cannot lock pag_send").clone();
        let pag_send3 = pag_send.lock().expect("cannot lock pag_send").clone();
        let pag_send4 = pag_send.lock().expect("cannot lock pag_send").clone();
        let pag_send6 = pag_send.lock().expect("cannot lock pag_send").clone();
        let pag_send7 = pag_send.lock().expect("cannot lock pag_send").clone();
        let pag_send8 = pag_send.lock().expect("cannot lock pag_send").clone();

        // read replayers from file (offline) or TCP stream (online)
        let readers = connect::make_readers(replay_source.clone(), worker.index(), worker.peers()).expect("couldn't create readers");

        worker.dataflow(|scope| {
            let pag: Stream<_, (PagEdge, Pair<u64, Duration>, isize)>  = pag::create_pag(scope, readers, index, 1);

            // log PAG to socket
            pag.inspect(move |(x, t, _)| {
                pag_send3
                    .send((t.first, PagData::Pag(x.clone())))
                    .expect("couldn't send pagedge")
            });

            let khops = pag.khops();

            // log khops edges to socket
            khops.inspect_time(move |t, ((x, _), hops)| {
                pag_send1
                    .send((t.first - 1, PagData::All((x.source.timestamp.as_nanos().try_into().unwrap(), x.destination.timestamp.as_nanos().try_into().unwrap(), *hops))))
                    .expect("khops_edges")
            });


            let khops_summary = khops.khops_summary();

            // log khops summary to socket
            khops_summary.inspect_time(move |t, ((a, wf, hops), (ac, wac))| {
                pag_send2
                    .send((t.first - 1, PagData::Agg(KHopSummaryData {a: *a, wf: *wf, ac: *ac, wac: *wac, hops: *hops})))
                    .expect("khops_summary")
            });


            let metrics = pag.metrics();

            // log metrics to socket
            metrics.inspect_time(move |t, x| {
                pag_send4
                    .send((t.first - 1, PagData::Met(MetricsData {
                        wf: x.0,
                        wt: x.1,
                        a: x.2,
                        ac: x.3,
                        at: x.4,
                        rc: x.5,
                    })))
                    .expect("metrics")
            });


            if let Some(epoch_max) = epoch_max {
                let max = Duration::from_millis(epoch_max);
                let max_nanos: u64 = max.as_nanos().try_into().unwrap();
                pag.max_epoch(max)
                    .inspect(move |(x, y)| {
                        pag_send6
                            .send((0, PagData::Inv(InvariantData::Epoch(EpochData {
                                max: max_nanos,
                                from: *x,
                                to: *y
                            }))))
                            .expect("inv_epoch")
                    });
            }

            if let Some(operator_max) = operator_max {
                let max = Duration::from_millis(operator_max);
                let max_nanos: u64 = max.as_nanos().try_into().unwrap();
                pag.max_operator(max)
                    .inspect(move |(x, y)| {
                        pag_send7
                            .send((0, PagData::Inv(InvariantData::Operator(OperatorData {
                                max: max_nanos,
                                from: x.clone(),
                                to: y.clone()
                            }))))
                            .expect("inv_op")
                    });
            }

            if let Some(message_max) = message_max {
                let max = Duration::from_millis(message_max);
                let max_nanos: u64 = max.as_nanos().try_into().unwrap();
                pag.max_message(max)
                    .inspect(move |x| {
                        pag_send8
                            .send((0, PagData::Inv(InvariantData::Message(MessageData {
                                max: max_nanos,
                                msg: x.clone(),
                            }))))
                            .expect("inv_msg")
                    });
            }
        });
    })
        .map_err(|x| STError(format!("error in the timely computation: {}", x)))?;

    Ok(())
}
