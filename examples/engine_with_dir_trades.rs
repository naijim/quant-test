use barter::{
    data::historical,
    engine::{trader::Trader, Engine},
    event::{Event, EventTx},
    execution::{
        simulated::{Config as ExecutionConfig, SimulatedExecution},
        Fees,
    },
    portfolio::{
        allocator::DefaultAllocator, portfolio::MetaPortfolio,
        repository::in_memory::InMemoryRepository, risk::DefaultRisk,
    },
    statistic::summary::{
        trading::{Config as StatisticConfig, TradingSummary},
        Initialiser,
    },
    strategy::example::{Config as StrategyConfig, RSIStrategy},
};
use barter_data::{
    event::{DataKind, MarketEvent},
    subscription::trade::PublicTrade,
};
use barter_integration::model::{
    instrument::{kind::InstrumentKind, Instrument},
    Exchange, Market, Side,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use parking_lot::Mutex;
use regex::Regex;
use serde::Deserialize;
use std::io::{prelude::*, BufReader};
use std::time::Instant;
use std::{collections::HashMap, fs, io, sync::Arc};
use std::{fs::File, path::PathBuf};
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let start = Instant::now();

    // Create channel to distribute Commands to the Engine & it's Traders (eg/ Command::Terminate)
    let (_command_tx, command_rx) = mpsc::channel(20);

    // Create Event channel to listen to all Engine Events in real-time
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let event_tx = EventTx::new(event_tx);

    // Generate unique identifier to associate an Engine's components
    let engine_id = Uuid::new_v4();

    // Create the Market(s) to be traded on (1-to-1 relationship with a Trader)
    let market = Market::new("binance", ("btc", "usdt", InstrumentKind::Spot));

    // Build global shared-state MetaPortfolio (1-to-1 relationship with an Engine)
    let portfolio = Arc::new(Mutex::new(
        MetaPortfolio::builder()
            .engine_id(engine_id)
            .markets(vec![market.clone()])
            .starting_cash(10_000.0)
            .repository(InMemoryRepository::new())
            .allocation_manager(DefaultAllocator {
                default_order_value: 100.0,
            })
            .risk_manager(DefaultRisk {})
            .statistic_config(StatisticConfig {
                starting_equity: 10_000.0,
                trading_days_per_year: 365,
                risk_free_return: 0.0,
            })
            .build_and_init()
            .expect("failed to build & initialise MetaPortfolio"),
    ));

    // Build Trader(s)
    let mut traders = Vec::new();

    // Create channel for each Trader so the Engine can distribute Commands to it
    let (trader_command_tx, trader_command_rx) = mpsc::channel(10);

    let data = load_binance_trade_init().unwrap();

    traders.push(
        Trader::builder()
            .engine_id(engine_id)
            .market(market.clone())
            .command_rx(trader_command_rx)
            .event_tx(event_tx.clone())
            .portfolio(Arc::clone(&portfolio))
            .data(historical::MarketFeed::new(data.into_iter()))
            .strategy(RSIStrategy::new(StrategyConfig { rsi_period: 14 }))
            .execution(SimulatedExecution::new(ExecutionConfig {
                simulated_fees_pct: Fees {
                    exchange: 0.1,
                    slippage: 0.05,
                    network: 0.0,
                },
            }))
            .build()
            .expect("failed to build trader"),
    );

    // Build Engine (1-to-many relationship with Traders)
    // Create HashMap<Market, trader_command_tx> so Engine can route Commands to Traders
    let trader_command_txs = HashMap::from([(market, trader_command_tx)]);

    let engine = Engine::builder()
        .engine_id(engine_id)
        .command_rx(command_rx)
        .portfolio(portfolio)
        .traders(traders)
        .trader_command_txs(trader_command_txs)
        .statistics_summary(TradingSummary::init(StatisticConfig {
            starting_equity: 1000.0,
            trading_days_per_year: 365,
            risk_free_return: 0.0,
        }))
        .build()
        .expect("failed to build engine");

    // Run Engine trading & listen to Events it produces
    tokio::spawn(listen_to_engine_events(event_rx));
    engine.run().await;

    let duration = start.elapsed();
    println!("Time elapsed is: {:?}", duration);
}

// Listen to Events that occur in the Engine. These can be used for updating event-sourcing,
// updating dashboard, etc etc.
async fn listen_to_engine_events(mut event_rx: mpsc::UnboundedReceiver<Event>) {
    while let Some(event) = event_rx.recv().await {
        match event {
            Event::Market(_) => {
                // Market Event occurred in Engine
            }
            Event::Signal(signal) => {
                // Signal Event occurred in Engine
                println!("{signal:?}");
            }
            Event::SignalForceExit(_) => {
                // SignalForceExit Event occurred in Engine
            }
            Event::OrderNew(new_order) => {
                // OrderNew Event occurred in Engine
                println!("{new_order:?}");
            }
            Event::OrderUpdate => {
                // OrderUpdate Event occurred in Engine
            }
            Event::Fill(fill_event) => {
                // Fill Event occurred in Engine
                println!("{fill_event:?}");
            }
            Event::PositionNew(new_position) => {
                // PositionNew Event occurred in Engine
                println!("{new_position:?}");
            }
            Event::PositionUpdate(updated_position) => {
                // PositionUpdate Event occurred in Engine
                println!("{updated_position:?}");
            }
            Event::PositionExit(exited_position) => {
                // PositionExit Event occurred in Engine
                println!("{exited_position:?}");
            }
            Event::Balance(balance_update) => {
                // Balance update Event occurred in Engine
                println!("{balance_update:?}");
            }
        }
    }
}

fn load_binance_trade_init(
) -> std::result::Result<Vec<MarketEvent<DataKind>>, MultipleFilesReaderError> {
    let mut reader = BinanceTradeReader::new(
        "/Users/jack/Workspace/crypto/data/binance/trade_tick",
        DateTime::<Utc>::from_naive_utc_and_offset(
            NaiveDate::from_ymd_opt(2019, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            Utc,
        ),
        DateTime::<Utc>::from_naive_utc_and_offset(
            NaiveDate::from_ymd_opt(2019, 11, 2)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            Utc,
        ),
    );
    reader.build_and_init()?;

    let res = reader
        .into_iter()
        .map(|trade| MarketEvent {
            exchange_time: DateTime::<Utc>::from_naive_utc_and_offset(
                NaiveDateTime::from_timestamp_millis(trade.4).unwrap(),
                Utc,
            ),
            received_time: Utc::now(),
            exchange: Exchange::from("binance"),
            instrument: Instrument::from(("btc", "usdt", InstrumentKind::Spot)),
            kind: DataKind::Trade(PublicTrade::from(trade)),
        })
        .collect();

    Ok(res)
}

#[derive(Error, Debug)]
enum MultipleFilesReaderError {
    #[error("try again")]
    TryAgain,

    #[error("no file")]
    NoFile,

    #[error("no more files")]
    NoMoreFiles,

    #[error("io error")]
    IOError(#[from] io::Error),
}

struct MultipleFilesReader {
    data_path: String,
    data_files: Option<Box<dyn Iterator<Item = String>>>,
    reader: Option<std::io::Lines<std::io::BufReader<File>>>,

    // for filtering filename with time range
    filter_func: Box<dyn Fn(&DateTime<Utc>, &DateTime<Utc>, &str) -> bool>,
    from_datetime: DateTime<Utc>,
    to_datetime: DateTime<Utc>,
}

impl MultipleFilesReader {
    pub fn new(
        value: &str,
        f: impl Fn(&DateTime<Utc>, &DateTime<Utc>, &str) -> bool + 'static,
        from_datetime: DateTime<Utc>,
        to_datetime: DateTime<Utc>,
    ) -> Self {
        Self {
            data_path: value.to_owned(),
            data_files: None,
            reader: None,

            filter_func: Box::new(f),
            from_datetime: from_datetime,
            to_datetime: to_datetime,
        }
    }

    pub fn build_and_init(&mut self) -> std::result::Result<(), MultipleFilesReaderError> {
        // get a list of all entries in the folder
        let entries = fs::read_dir(self.data_path.as_str())?;

        // extract the filenames from the directory entries and store them in a vector
        let mut file_names: Vec<String> = entries
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.is_file() {
                    path.file_name()?.to_str().map(|s| s.to_owned())
                } else {
                    None
                }
            })
            .collect();

        // sort file names in ascending order
        file_names.sort();
        self.data_files = Some(Box::new(file_names.into_iter()));

        // open first file if any
        self.load_next_file()?;

        return Ok(());
    }

    fn find_next_file(&mut self) -> std::result::Result<String, MultipleFilesReaderError> {
        loop {
            match &mut self.data_files {
                Some(files) => match files.next() {
                    Some(filename) => {
                        if (*self.filter_func)(&self.from_datetime, &self.to_datetime, &filename) {
                            let path = PathBuf::from(&self.data_path).join(&filename);
                            return Ok(path.as_path().display().to_string());
                        } else {
                            continue;
                        }
                    }
                    None => return Err(MultipleFilesReaderError::NoMoreFiles),
                },
                None => return Err(MultipleFilesReaderError::NoFile),
            }
        }
    }

    // if no error, readline returns next line or eof
    fn load_next_file(&mut self) -> std::result::Result<(), MultipleFilesReaderError> {
        match self.find_next_file() {
            Ok(filename) => {
                self.reader = Some(BufReader::new(File::open(filename)?).lines());
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }

    fn readline(&mut self) -> std::result::Result<String, MultipleFilesReaderError> {
        match &mut self.reader {
            Some(reader) => match reader.next() {
                Some(line) => match line {
                    Ok(line) => return Ok(line),
                    Err(e) => return Err(MultipleFilesReaderError::IOError(e)),
                },
                None => match self.load_next_file() {
                    Ok(()) => return Err(MultipleFilesReaderError::TryAgain),
                    Err(e) => return Err(e),
                },
            },
            None => return Err(MultipleFilesReaderError::NoFile),
        }
    }
}

impl Iterator for MultipleFilesReader {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.readline() {
                Ok(line) => return Some(line),
                Err(e) => match e {
                    MultipleFilesReaderError::TryAgain => continue,
                    MultipleFilesReaderError::NoFile
                    | MultipleFilesReaderError::NoMoreFiles
                    | MultipleFilesReaderError::IOError(_) => return None,
                },
            };
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
struct BinanceTradeRecord(u64, f64, f64, f64, i64, String, String);

impl From<BinanceTradeRecord> for PublicTrade {
    fn from(item: BinanceTradeRecord) -> Self {
        PublicTrade {
            id: item.0.to_string(),
            price: item.1,
            amount: item.2,
            side: if item.5 == "True" {
                Side::Buy
            } else {
                Side::Sell
            },
        }
    }
}

struct BinanceTradeReader {
    data_path: String,

    from_date: DateTime<Utc>,
    to_date: DateTime<Utc>,

    files_reader_iter: Option<Box<dyn Iterator<Item = String>>>,
}

impl BinanceTradeReader {
    pub fn new(data_path: &str, from_date: DateTime<Utc>, to_date: DateTime<Utc>) -> Self {
        Self {
            data_path: data_path.to_owned(),
            from_date: from_date,
            to_date: to_date,
            files_reader_iter: None,
        }
    }

    pub fn build_and_init(&mut self) -> std::result::Result<(), MultipleFilesReaderError> {
        let mut reader = MultipleFilesReader::new(
            &self.data_path,
            BinanceTradeReader::filter_data_file,
            self.from_date.clone(),
            self.to_date.clone(),
        );

        match reader.build_and_init() {
            Ok(_) => self.files_reader_iter = Some(Box::new(reader.into_iter())),
            Err(e) => return Err(e),
        };

        return Ok(());
    }

    // filename pattern: BTCUSDT-trades-2019-10.csv
    // https://stackoverflow.com/questions/73796125/how-to-get-the-start-and-the-end-of-date-for-each-month-with-naivedate-rust
    pub fn filter_data_file(
        from_datetime: &DateTime<Utc>,
        to_datetime: &DateTime<Utc>,
        filename: &str,
    ) -> bool {
        let re = Regex::new(r#"\w+-\w+-(\d+)-(\d+)\.(\w+)"#).unwrap();
        let file_time = re.captures(filename).and_then(|cap| {
            if cap.len() != 4 || cap.get(3).unwrap().as_str() != "csv" {
                return None;
            }

            let year = cap.get(1).unwrap().as_str().parse::<i32>().unwrap();
            let month = cap.get(2).unwrap().as_str().parse::<u32>().unwrap();

            let start_datetime_of_month = DateTime::<Utc>::from_naive_utc_and_offset(
                NaiveDate::from_ymd_opt(year, month, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                Utc,
            );

            let end_datetime_of_month = DateTime::<Utc>::from_naive_utc_and_offset(
                NaiveDate::from_ymd_opt(year, month + 1, 1)
                    .unwrap_or(NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap())
                    .pred_opt()
                    .unwrap()
                    .and_hms_opt(23, 59, 59)
                    .unwrap(),
                Utc,
            );

            return Some((start_datetime_of_month, end_datetime_of_month));
        });

        if let Some((file_from_time, file_to_time)) = file_time {
            if file_to_time < *from_datetime {
                return false;
            }

            if file_from_time > *to_datetime {
                return false;
            }
        } else {
            return false;
        }

        return true;
    }

    fn filter_data(&self, record: &BinanceTradeRecord) -> bool {
        if let Some(time_without_tz) = NaiveDateTime::from_timestamp_millis(record.4) {
            let record_time = DateTime::<Utc>::from_naive_utc_and_offset(time_without_tz, Utc);
            if record_time >= self.from_date && record_time <= self.to_date {
                return true;
            } else {
                return false;
            }
        } else {
            return false;
        };
    }
}

impl Iterator for BinanceTradeReader {
    type Item = BinanceTradeRecord;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut iter) = self.files_reader_iter {
                match iter.next() {
                    Some(x) => match csv_line::from_str::<BinanceTradeRecord>(&x) {
                        Ok(e) => {
                            if self.filter_data(&e) {
                                return Some(e);
                            } else {
                                continue;
                            }
                        }

                        // TODO: parse failure, log error
                        Err(_) => continue,
                    },
                    None => return None,
                }
            }
        }
    }
}
