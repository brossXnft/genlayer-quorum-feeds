# v0.3.0
# { "Depends": "py-genlayer:babwp9t37jw9g4j2bccgn14g6x7x9s1f7dsmh7emvz735pz4zxa0" }

# Reads a Quorum Feeds value (a Rust contract) from Python and refuses stale data.
# Deploy with the Quorum Feeds address, then call snapshot("BTC-USD", 3600).
import typing

import genlayer as gl
from genlayer.types import *


class FeedConsumer(gl.contract.Contract):
	oracle: Address
	last_feed: str
	last_value: u256
	last_text: str
	last_at: u256

	def __init__(self, oracle: str):
		self.oracle = Address(oracle)

	@gl.public.write
	def snapshot(self, feed: str, max_age_s: int) -> str:
		# latest_fresh raises if the feed has not been updated within max_age_s.
		r = gl.contract.get_at(self.oracle).view().latest_fresh(feed, max_age_s)
		self.last_feed = feed
		self.last_value = u256(r['value'])
		self.last_text = r['text']
		self.last_at = u256(r['at'])
		return f"{feed} = {r['text']} {r['unit']} (round {r['round']}, {r['kept']} sources)"

	@gl.public.view
	def get_last(self) -> dict[str, typing.Any]:
		return {
			'feed': self.last_feed,
			'value': self.last_value,
			'text': self.last_text,
			'at': self.last_at,
		}
