use std::{
	collections::HashMap,
	fmt::{self, Display},
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
};

use bitcoin::{FeeRate, Transaction, Txid};
use lightning_block_sync::{
	http::JsonResponse,
	rest::RestClient,
	rpc::{RpcClient, RpcError},
	BlockSource,
};

use crate::chain::bitcoind_rpc::{rpc_credentials, GetMempoolEntryResponse};

use super::bitcoind_rpc::{
	endpoint, FeeRateEstimationMode, FeeResponse, GetRawMempoolResponse, GetRawTransactionResponse,
	MempoolEntry, MempoolMinFeeResponse,
};

pub struct BitcoindRestClient {
	rest_client: Arc<RestClient>,
	rpc_client: Arc<RpcClient>,
	latest_mempool_timestamp: AtomicU64,
	mempool_entries_cache: tokio::sync::Mutex<HashMap<Txid, MempoolEntry>>,
	mempool_txs_cache: tokio::sync::Mutex<HashMap<Txid, (Transaction, u64)>>,
}

impl BitcoindRestClient {
	/// Creates a new, primarily REST client for the chain interactions
	/// with Bitcoin Core.
	///
	/// Aside the required REST host and port, we must provide RPC
	/// configuration options for necessary calls not supported by the
	/// REST interface.
	pub(crate) fn new(
		rest_host: String, rest_port: u16, rpc_host: String, rpc_port: u16, rpc_user: String,
		rpc_password: String,
	) -> Self {
		let rest_endpoint = endpoint(rest_host, rest_port).with_path("/rest".to_string());
		let rest_client = Arc::new(RestClient::new(rest_endpoint));

		let rpc_endpoint = endpoint(rpc_host, rpc_port);
		let rpc_credentials = rpc_credentials(rpc_user, rpc_password);
		let rpc_client = Arc::new(RpcClient::new(&rpc_credentials, rpc_endpoint));

		let latest_mempool_timestamp = AtomicU64::new(0);

		let mempool_entries_cache = tokio::sync::Mutex::new(HashMap::new());
		let mempool_txs_cache = tokio::sync::Mutex::new(HashMap::new());

		Self {
			rest_client,
			rpc_client,
			latest_mempool_timestamp,
			mempool_entries_cache,
			mempool_txs_cache,
		}
	}

	pub(crate) fn rest_client(&self) -> Arc<RestClient> {
		Arc::clone(&self.rest_client)
	}

	/// Broadcasts the provided transaction.
	///
	/// We rely on the internal RPC client to make this call, as this
	/// operation is not supported by Bitcoin Core's REST interface.
	pub(crate) async fn broadcast_transaction(&self, tx: &Transaction) -> std::io::Result<Txid> {
		let tx_serialized = bitcoin::consensus::encode::serialize_hex(tx);
		let tx_json = serde_json::json!(tx_serialized);
		self.rpc_client.call_method::<Txid>("sendrawtransaction", &[tx_json]).await
	}

	/// Retrieve the fee estimate needed for a transaction to begin
	/// confirmation within the provided `num_blocks`.
	///
	/// We rely on the internal RPC client to make this call, as this
	/// operation is not supported by Bitcoin Core's REST interface.
	pub(crate) async fn get_fee_estimate_for_target(
		&self, num_blocks: usize, estimation_mode: FeeRateEstimationMode,
	) -> std::io::Result<FeeRate> {
		let num_blocks_json = serde_json::json!(num_blocks);
		let estimation_mode_json = serde_json::json!(estimation_mode);
		self.rpc_client
			.call_method::<FeeResponse>(
				// TODO(@enigbe): Move FeeResponse to mod.rs
				"estimatesmartfee",
				&[num_blocks_json, estimation_mode_json],
			)
			.await
			.map(|resp| resp.0)
	}

	/// Retrieves the minimum mempool fee rate.
	pub async fn get_mempool_minimum_fee_rate(&self) -> std::io::Result<FeeRate> {
		// TODO(@enigbe): Move MempoolMinFeeResponse to common.
		self.rest_client
			.request_resource::<JsonResponse, MempoolMinFeeResponse>("mempool/info.json")
			.await
			.map(|resp| resp.0)
	}

	/// Retrieves the raw transaction for the provided transaction ID.
	pub(crate) async fn get_raw_transaction(
		&self, txid: &Txid,
	) -> std::io::Result<Option<Transaction>> {
		let txid_hex = bitcoin::consensus::encode::serialize_hex(txid);
		let tx_path = format!("tx/{}.json", txid_hex);

		match self
			.rest_client
			.request_resource::<JsonResponse, GetRawTransactionResponse>(&tx_path)
			.await
		{
			Ok(resp) => Ok(Some(resp.0)),
			Err(e) => match e.kind() {
				std::io::ErrorKind::Other => {
					let http_error_res: Result<Box<HttpError>, _> = e.downcast();
					match http_error_res {
						Ok(http_error) => {
							if &http_error.status_code == "404" {
								Ok(None)
							} else {
								Err(std::io::Error::new(std::io::ErrorKind::Other, http_error))
							}
						},
						Err(_) => {
							let error_msg = format!("Failed to process {} response.", tx_path);
							Err(std::io::Error::new(std::io::ErrorKind::Other, error_msg.as_str()))
						},
					}
				},
				_ => {
					let error_msg = format!("Failed to process {} response.", tx_path);
					Err(std::io::Error::new(std::io::ErrorKind::Other, error_msg.as_str()))
				},
			},
		}
	}

	/// Retrieves the transaction IDs in the mempool.
	pub(crate) async fn get_raw_mempool(&self) -> std::io::Result<Vec<Txid>> {
		self.rest_client
			.request_resource::<JsonResponse, GetRawMempoolResponse>(
				"mempool/contents.json?verbose=false",
			)
			.await
			.map(|resp| resp.0)
	}

	/// Retrieves a mempool entry if it exists.
	///
	/// We rely on the internal RPC client to make this call because
	/// using the REST interface is not as efficient. With REST, we have
	/// to retrieve the list of entries in the mempool and then filter
	/// for the txid of interest.
	pub(crate) async fn get_mempool_entry(
		&self, txid: Txid,
	) -> std::io::Result<Option<MempoolEntry>> {
		let txid_hex = bitcoin::consensus::encode::serialize_hex(&txid);
		let txid_json = serde_json::json!(txid_hex);

		match self
			.rpc_client
			.call_method::<GetMempoolEntryResponse>("getmempoolentry", &[txid_json])
			.await
		{
			Ok(resp) => Ok(Some(MempoolEntry { txid, time: resp.time, height: resp.height })),
			Err(e) => match e.into_inner() {
				Some(inner) => {
					let rpc_error_res: Result<Box<RpcError>, _> = inner.downcast();

					match rpc_error_res {
						Ok(rpc_error) => {
							// Check if it's the 'not found' error code.
							if rpc_error.code == -5 {
								Ok(None)
							} else {
								Err(std::io::Error::new(std::io::ErrorKind::Other, rpc_error))
							}
						},
						Err(_) => Err(std::io::Error::new(
							std::io::ErrorKind::Other,
							"Failed to process getmempoolentry response",
						)),
					}
				},
				None => Err(std::io::Error::new(
					std::io::ErrorKind::Other,
					"Failed to process getmempoolentry response",
				)),
			},
		}
	}

	pub(crate) async fn update_mempool_entries_cache(&self) -> std::io::Result<()> {
		let mempool_txids = self.get_raw_mempool().await?;

		let mut mempool_entries_cache = self.mempool_entries_cache.lock().await;
		mempool_entries_cache.retain(|txid, _entry| mempool_txids.contains(txid));

		if let Some(difference) = mempool_txids.len().checked_sub(mempool_entries_cache.capacity())
		{
			mempool_entries_cache.reserve(difference);
		}

		for txid in mempool_txids {
			if mempool_entries_cache.contains_key(&txid) {
				continue;
			}

			if let Some(entry) = self.get_mempool_entry(txid).await? {
				mempool_entries_cache.insert(txid, entry.clone());
			}
		}

		mempool_entries_cache.shrink_to_fit();

		Ok(())
	}

	pub(crate) async fn get_mempool_transactions_and_timestamp_at_height(
		&self, best_processed_height: u32,
	) -> std::io::Result<Vec<(Transaction, u64)>> {
		let prev_mempool_time = self.latest_mempool_timestamp.load(Ordering::Relaxed);
		let mut latest_time = prev_mempool_time;

		self.update_mempool_entries_cache().await?;

		let mempool_entries_cache = self.mempool_entries_cache.lock().await;
		let mut mempool_txs_cache = self.mempool_txs_cache.lock().await;
		mempool_txs_cache.retain(|txid, _entry| mempool_entries_cache.contains_key(txid));

		if let Some(difference) =
			mempool_entries_cache.len().checked_sub(mempool_txs_cache.capacity())
		{
			mempool_txs_cache.reserve(difference);
		}

		let mut txs_to_emit = Vec::with_capacity(mempool_entries_cache.len());
		for (txid, entry) in mempool_entries_cache.iter() {
			if entry.time > latest_time {
				latest_time = entry.time;
			}

			// Avoid emitting transactions that are already emitted if we can guarantee
			// blocks containing ancestors are already emitted. The bitcoind rpc interface
			// provides us with the block height that the tx is introduces to the mempool.
			// If we have already emitted the block of height, we can assume that all
			// ancestor txs have been processed by the receiver.
			let ancestor_within_height = entry.height <= best_processed_height;
			let is_already_emitted = entry.time <= prev_mempool_time;
			if is_already_emitted && ancestor_within_height {
				continue;
			}

			if let Some((cached_tx, cached_time)) = mempool_txs_cache.get(txid) {
				txs_to_emit.push((cached_tx.clone(), *cached_time));
				continue;
			}

			match self.get_raw_transaction(&entry.txid).await {
				Ok(Some(tx)) => {
					mempool_txs_cache.insert(entry.txid, (tx.clone(), entry.time));
					txs_to_emit.push((tx, entry.time));
				},
				Ok(None) => {
					continue;
				},
				Err(e) => return Err(e),
			}
		}

		if !txs_to_emit.is_empty() {
			self.latest_mempool_timestamp.store(latest_time, Ordering::Release);
		}

		Ok(txs_to_emit)
	}
}

impl BlockSource for BitcoindRestClient {
	fn get_header<'a>(
		&'a self, header_hash: &'a bitcoin::BlockHash, height_hint: Option<u32>,
	) -> lightning_block_sync::AsyncBlockSourceResult<'a, lightning_block_sync::BlockHeaderData> {
		Box::pin(async move { self.rest_client.get_header(header_hash, height_hint).await })
	}

	fn get_block<'a>(
		&'a self, header_hash: &'a bitcoin::BlockHash,
	) -> lightning_block_sync::AsyncBlockSourceResult<'a, lightning_block_sync::BlockData> {
		Box::pin(async move { self.rest_client.get_block(header_hash).await })
	}

	fn get_best_block(
		&self,
	) -> lightning_block_sync::AsyncBlockSourceResult<(bitcoin::BlockHash, Option<u32>)> {
		Box::pin(async move { self.rest_client.get_best_block().await })
	}
}

#[derive(Debug)]
struct HttpError {
	pub(crate) status_code: String,
	pub(crate) contents: Vec<u8>,
}

impl std::error::Error for HttpError {}

impl Display for HttpError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let contents = String::from_utf8_lossy(&self.contents);
		write!(f, "status_code: {}, contents: {}", self.status_code, contents)
	}
}
