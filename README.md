# The First Nonce: on-chain program

Source of the Solana program deployed at [`GUW16mLqFWfGjD5AFFSwDxQyGAFSDvPdEbryVa98TGs3`](https://solscan.io/account/GUW16mLqFWfGjD5AFFSwDxQyGAFSDvPdEbryVa98TGs3).

The deployed binary is built reproducibly with `solana-verify` (see `.github/workflows/verifiable-build.yml`), so anyone can check it matches this code:

```bash
solana-verify verify-from-repo -um --program-id GUW16mLqFWfGjD5AFFSwDxQyGAFSDvPdEbryVa98TGs3 https://github.com/thefirstnonce/first-nonce-program --library-name first_nonce --base-image solanafoundation/solana-verifiable-build@sha256:2e0b78f44ee76612e9260c7c988570c5e14de6fbd93e0ab07115ec7054473b4f
```

Unaudited. The program is upgradeable; the upgrade authority is `Ayq6tRC3atGKuKP6wdt4fX1fSBe1wnBrzMAkCxMatCbG`. This repo contains only the on-chain program and its IDL (`idl/first_nonce.json`).

---


An Anchor program (anchor 1.2, Solana / Agave 4.x). A new coin's opening supply can't be bought. It can only be mined with proof-of-work. Trading then opens on a single-sided, permanently locked Meteora DAMM v2 pool.

> **Unaudited.** Keep the upgrade authority until it has been reviewed.

- **Program id:** `GUW16mLqFWfGjD5AFFSwDxQyGAFSDvPdEbryVa98TGs3`. The deploy keypair is `target/deploy/first_nonce-keypair.json`. Back it up, since it is the program address.
- **Mechanics spec:** `../docs/mechanics.md`.
- **Meteora verification notes:** `../docs/meteora-notes.md`.

```
launch ──> the dig (30/60/90 blocks of 10 s, mined only) ──> open (Meteora pool, locked) ──> claim / collect
```

## Layout

```
program/
  Anchor.toml, Cargo.toml
  programs/first-nonce/src/
    lib.rs            instruction entry points
    constants.rs      seeds, supply, defaults, external ids
    state.rs          Config, Coin (+ block state machine), Miner, FeeSchedule
    mining.rs         share hash, retarget, rewards, sqrt price math (+ unit tests)
    funds.rs          open reserve sizing and the refunds to the launcher (+ unit test)
    cp_amm.rs         hand-written CPI into Meteora DAMM v2
    metadata.rs       hand-written CPI into Metaplex CreateMetadataAccountV3 (classic mints only)
    events.rs
    instructions/     admin, launch, submit, crank, open, claim (+ seal), collect
  sdk/                TypeScript SDK (ESM) + test-vectors.json
```

## Numbers

| | value |
|---|---|
| total supply | 1,000,000,000 (6 decimals), no freeze authority. Default: a **Token-2022 mint with its metadata in the mint** (metadata pointer + token metadata extensions, both immutable). A classic SPL mint with immutable Metaplex metadata is still available (pass the classic token program to `launch`). |
| mined (opening) supply | 20% = 200,000,000 |
| pool supply | 80% = 800,000,000, deposited single-sided at open. About 0.001 token of rounding is burned. |
| dev allocation | none |
| block time | 10 s. Dig of 5 / 10 / 15 min = 30 / 60 / 90 blocks |
| reward per block | 200,000,000 / blocks, flat. The remainder goes on the last block. |
| share target S | 400 shares per block (config `share_target`) |
| starting target | `2^46`, i.e. 2^18 = 262,144 expected hashes per share. The SDK's plain-JS reference loop measured ~210 kH/s on one server core, which is about one share every 1.25 s. Wasm or WebGPU miners are much faster. |
| retarget | every block: `next = target × S × duration / (shares × 10)`, clamped to 4x per block. An empty block gets 4x easier. |
| late-share grace | a share for block n still counts until 3 s after n's window, unless the block was already closed |
| opening market cap | 30 SOL fully diluted by default (config `default_open_fdv_lamports`). The launcher may pick 10–1,000 SOL. |
| pool fee | 25% at open, minus 3% every 30 s, reaching 1% after 4 minutes (Meteora linear time scheduler). Fees are taken in SOL only. |
| fee split | Meteora keeps 20% of each trading fee. Our locked position gets 80%, and `collect` pays 80% of that to the launcher and 20% to the treasury. |
| launch fee | config `launch_fee_lamports` to the treasury: **0.005 SOL on mainnet** since 2026-10-13 (`update_config`; init default 0.05) |
| open reserve | priced at launch from the Rent sysvar (`funds::open_reserve`): every account `open` creates (cp-amm pool, position, position NFT mint and account, both pool token vaults, the vault's coin and wSOL token accounts) + 1 lamport of wSOL + one token-account rent kept for `collect` + a 0.001 SOL margin. **0.021991 SOL** for a Token-2022 coin at today's mainnet rent (5,080 lamports/byte). `open` returns the unused part (0.002488 SOL: the margin and the wSOL account, which `open` closes) to the launcher; 0.001488 SOL stays in the vault for `collect`. |

**Launcher cost per launch (measured in the e2e test at today's mainnet rent, 0.005 SOL fee):** **0.041783 SOL** upfront including the 10,000-lamport two-signature tx fee: 0.005 fee + 0.012283 coin account rent (2,290 bytes) + ~0.0025–0.0027 mint rent (318 bytes + name + symbol + uri; 0.002499 for a 364-byte mint) + 0.021991 reserve. 0.002488 comes back at `open`, so the net is **~0.0393 SOL**. Before this change it was 0.112 SOL (0.05 fee, a flat 0.035 reserve of which 0.0185 stayed stranded, Metaplex metadata rent 0.003734 + its 0.01 fee), and the `open` cranker paid ~0.003 SOL of token-account rent. A classic (Metaplex) launch now costs 0.054059 SOL upfront at the 0.005 fee.

**Whoever calls `open` pays only the transaction fee.** The vault pays every rent from the reserve.

**Miner cost:** one transaction fee per submission of up to 8 shares (5,000 lamports plus any priority fee). The first submission also pays ~0.00137 SOL of miner-account rent (today's mainnet rent), which comes back at claim.

## The share hash (the browser miner must match this byte for byte)

```
challenge = keccak256( mint[32] || u64_le(block_number)[8] || slot_hash[32] )        72-byte preimage
share     = keccak256( challenge[32] || beneficiary[32] || u64_le(nonce)[8] )        72-byte preimage
value     = u64 big-endian of share[0..8]
valid     = value < coin.target
```

- `keccak256` is original Keccak-256 (padding byte `0x01`, rate 136), the same as Ethereum and Solana's `sol_keccak256` syscall. **It is not NIST SHA3-256** (padding `0x06`). js-sha3 `keccak256` is correct; `sha3_256` is wrong.
- `mint` is the coin's mint pubkey, 32 raw bytes.
- `block_number` is the block index (0-based) as a u64, little-endian (8 bytes), even though the account stores it as u32.
- `slot_hash` is the newest entry in the SlotHashes sysvar when the block opened (account data bytes `[16..48]`). The miner never computes this: it reads `coin.challenge` from the Coin account.
- `beneficiary` is the wallet that gets credited, 32 raw bytes. Shares can only ever pay this key, so a relayer can submit them but can't redirect them.
- `nonce` is a u64, little-endian (8 bytes). Within a block, each beneficiary's nonces must be strictly increasing across all submissions, so scan upward from 0 at each new block.
- `target` is `coin.target` (u64). Expected hashes per share = `2^64 / target`.

**Byte offsets of the 72-byte share preimage:**

| bytes | content |
|---|---|
| 0–31 | challenge |
| 32–63 | beneficiary |
| 64–71 | nonce, LE |

Only the last 8 bytes change per attempt, so a GPU kernel can keep bytes 0–63 constant.

**Test vectors:** `sdk/test-vectors.json` has 4 challenges, 72 shares, and one real share found at the start target.

- Generated with js-sha3.
- Re-checked with an independent Python Keccak.
- Checked by the program's own Rust test (`mining::tests::matches_sdk_test_vectors`).

## PDAs and addresses

**Our program** (`GUW16m…`):

| account | seeds | notes |
|---|---|---|
| Config | `["config"]` | global |
| Coin | `["coin", mint]` | per coin; also the mint authority and the metadata update authority |
| vault | `["vault", coin]` | system-owned, no data. Holds the open reserve, owns the pool-side token accounts, and is the owner/creator of the locked Meteora position. |
| Miner | `["miner", coin, beneficiary]` | per miner per coin |
| position NFT mint | `["position_nft", coin]` | created by cp-amm as a Token-2022 mint |
| fee account | `["fee", coin]` | temporary wSOL account, created and closed inside `collect` |

**Other programs:**

| account | derivation |
|---|---|
| vault coin ATA | ATA(vault, mint) |
| vault wSOL ATA | ATA(vault, So111…112) |
| Metaplex metadata | `["metadata", metaqbx…, mint]` under Token Metadata |
| Meteora pool | `["cpool", max(mint, wSOL), min(mint, wSOL)]` under cp-amm (byte-wise max/min) |
| Meteora position | `["position", position_nft_mint]` |
| Meteora position NFT account | `["position_nft_account", position_nft_mint]` |
| Meteora token vaults | `["token_vault", mint, pool]` and `["token_vault", wSOL, pool]` |
| cp-amm pool authority | `HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC` |
| cp-amm event authority | `3rmHSu74h1ZcmAisVcWerTCiRDQbUrBKmcwptYGjHfet` |

`sdk/src/pda.ts` `coinAddresses(mint)` derives all of these.

## Instructions

Discriminators are anchor's `sha256("global:<name>")[..8]` and are listed in `sdk/src/constants.ts`. Account order below is the order the program expects.

### `init_config(admin: Pubkey, treasury: Pubkey)`

Run once, by the program's **upgrade authority** only. It checks the ProgramData account, so nobody can front-run it. Creates `Config` with the defaults above.

Accounts: `authority (w,s)`, `config (w)`, `program_data`, `system_program`.

### `update_config(args: UpdateConfigArgs)`

Admin only. Every field is optional:

- admin, treasury
- launch fee, launcher fee bps
- paused (stops new launches)
- share target S, start target
- default / min / max opening FDV
- the pool fee schedule

The FDV range and the fee schedule are validated against Meteora's limits. Coins already launched keep the values snapshotted at their launch.

Accounts: `admin (s)`, `config (w)`.

### `launch(args: { name, symbol, uri, dig_minutes: 5|10|15, open_fdv_lamports: u64 (0 = default) })`

1. Pays the launch fee to the treasury and the open reserve (`funds::open_reserve` at the current rent) to the vault.
2. Creates the mint. It must be a fresh keypair that signs the transaction: 6 decimals, mint authority = Coin PDA, no freeze authority. Name ≤ 32 bytes, symbol ≤ 10 bytes, uri ≤ 200 bytes.
   - `token_program` = Token-2022 (the SDK default): the mint gets the metadata pointer extension (pointing at the mint itself, no authority) and the token metadata extension (name, symbol, uri), whose update authority is then removed, so it can never change. No Metaplex account is created. Meteora cp-amm accepts these two extensions without a token badge (`is_permissionless_supported_mint`).
   - `token_program` = classic SPL Token: a classic mint plus immutable Metaplex metadata (update authority = Coin PDA), as before.
4. Opens block 0 with a challenge built from the current slot hash.

The dig starts immediately and lasts `dig_minutes`. No tokens exist yet.

Accounts: `launcher (w,s)`, `config (w)`, `mint (w,s)`, `coin (w)`, `vault (w)`, `treasury (w)`, `metadata (w)`, `token_metadata_program`, `slot_hashes`, `token_program`, `system_program`.

### `submit_shares(block: u32, nonces: Vec<u64>)`

Anyone can submit 1–8 nonces for one beneficiary. The miner account is created on the first submission, paid by `payer`.

`block` must be the open block, the coin must be digging, and `now < block_end + 3 s`. Every nonce must:

- be `>= miner.next_nonce` (strictly increasing), and
- hash below `coin.target`.

Otherwise the whole transaction fails. Any earlier, already-closed block of this miner is settled into `pending` first.

A submission for a closed block (already closed, or past its window + grace) fails with `BlockClosed`; one for a future block fails with `BlockNotOpen`. A miner never pays for a submission that credits nothing, and a client's preflight catches it before sending. A late submission no longer closes a block past its grace: that is `crank`'s job (the site's miner cranks after the grace).

A live submission then runs the lazy block close (see `crank`), so a late share inside the grace both counts and closes its block.

**Relayers:** send one beneficiary per transaction. If one transaction carries submissions for two beneficiaries and lands inside the grace, the first instruction closes the block, the second one is stale, and the whole transaction fails.

The `SharesSubmitted` event carries the beneficiary, block, accepted nonces, challenge and target, so anyone can recompute every share hash from the event alone (`parseEvents` in the SDK).

Accounts: `payer (w,s)`, `coin (w)`, `beneficiary`, `miner (w)`, `slot_hashes`, `system_program`.

### `crank()`

Permissionless. For every block whose 10 s window has passed:

1. **Close it.** `tokens_per_share[b] = (reward_b + carry) / shares_b`. The rounding dust rolls into `carry`. If shares = 0, the whole reward rolls into `carry`.
2. **Retarget** on the time the block could accept shares: from its open to `min(now, window end + 3 s)`. A block closed late because nobody cranked does not look slow. The target it was mined at goes to `block_targets[b]`.
3. **Mark skipped blocks.** Blocks nobody touched are empty, and their reward rolls forward too.
4. **Open the next block** with `challenge = keccak256(mint || u64_le(n) || newest slot hash)`.

After the last block the coin moves to `ENDED`. Any carry left over becomes `unmined` and is never minted.

Accounts: `coin (w)`, `slot_hashes`.

### `open()`

Permissionless, once the last block's window has passed. It runs the lazy close first, so no separate crank is needed.

1. Creates the vault's coin ATA (under the coin's token program) and wSOL ATA, paid by the vault.
2. Mints 800,000,000 to the vault's coin ATA, and funds the vault's wSOL ATA with 1 lamport from the vault, which cp-amm requires.
3. CPI `initialize_customizable_pool`:
   - coin / wSOL
   - `sqrt_price = sqrt_min_price = sqrt(open_fdv / 1e15 lamports per raw unit)` in Q64.64, `sqrt_max_price = MAX`
   - `liquidity = 800e12 × sqrt_price`
   - OnlyB fees, timestamp activation, active now
   - the coin's fee schedule
   - The vault PDA is the payer and creator. The position NFT mint is our PDA.
4. CPI `permanent_lock_position(liquidity)`. All of the liquidity is locked forever.
5. Burns the ~0.001-token rounding remainder.
6. Closes the (now empty) wSOL ATA back into the vault.
7. If the coin's launcher is passed as the first remaining account (writable), sends everything in the vault above one token-account rent to it (`ReserveRefunded`). Otherwise that happens at the next `collect`.
8. Sets state to `OPENED`.

The cranker pays only the transaction fee.

Compute limit: `COMPUTE_UNITS.open` (330k; measured 229k–261k).

Accounts: `cranker (w,s)`, `coin (w)`, `mint (w)`, `vault (w)`, `vault_token_a (w)`, `vault_wsol (w)`, `native_mint`, `position_nft_mint (w)`, `position_nft_account (w)`, `pool (w)`, `position (w)`, `token_a_vault (w)`, `token_b_vault (w)`, `pool_authority`, `event_authority`, `cp_amm_program`, `slot_hashes`, `token_program`, `token_2022_program`, `associated_token_program`, `system_program`, then optionally `launcher (w)`. `token_program` is always the classic program (wSOL); the coin's own program is `token_2022_program` for Token-2022 coins.

### `claim()`

Permissionless, after `open` only. Nobody holds a token before the pool exists, so nobody can create a rival pool first.

1. Settles the miner's last block.
2. Mints `pending` to the beneficiary's ATA (under the coin's token program; 170 bytes for Token-2022), which is created if needed and paid by `payer`.
3. Closes the miner account and sends its rent to whoever paid it.

Accounts: `payer (w,s)`, `coin (w)`, `mint (w)`, `beneficiary`, `miner (w)`, `rent_payer (w)`, `beneficiary_ata (w)`, `token_program`, `associated_token_program`, `system_program`.

### `collect()`

Permissionless crank.

1. Creates a temporary wSOL account at `["fee", coin]`, paid from the vault.
2. CPI `claim_position_fee`, signed by the vault.
3. Closes the temporary account into the vault, which unwraps the fees.
4. Sends `launcher_fee_bps` (80%) of the fee amount to the launcher and the rest to `config.treasury`, in native SOL. Anything else in the vault above one token-account rent (unused reserve) goes to the launcher in the same transfer (`ReserveRefunded`). This also returns what coins launched before this change left in their vaults (0.0185 SOL on the mainnet canary).

Each recipient must be able to hold the transfer. A totally empty wallet can't receive less than ~0.00089 SOL, so `collect` fails until fees build up. Nothing is lost: uncollected fees stay in the position.

Accounts: `cranker (s)`, `config`, `coin (w)`, `mint`, `vault (w)`, `vault_token_a (w)`, `fee_account (w)`, `native_mint`, `launcher (w)`, `treasury (w)`, `pool_authority`, `pool`, `position (w)`, `token_a_vault (w)`, `token_b_vault (w)`, `position_nft_account`, `event_authority`, `cp_amm_program`, `token_program`, `system_program`, then the Token-2022 program for Token-2022 coins.

### `seal()`

Permissionless, once `claimed == allocated`: every earned token has been minted. It removes the mint authority for good, so supply is fixed on chain.

If some miners never claim, the mint authority stays with the Coin PDA. The program can still only mint what was allocated.

Accounts: `coin`, `mint (w)`, `token_program`.

## Account layouts

All accounts use anchor borsh: an 8-byte discriminator followed by the fields in order. Sizes include the discriminator. Decoders are in `sdk/src/accounts.ts`.

### Config (218 bytes)

| field | type |
|---|---|
| admin | Pubkey |
| treasury | Pubkey |
| launch_fee_lamports | u64 |
| launcher_fee_bps | u16 |
| paused | bool |
| share_target | u32 |
| start_target | u64 |
| default_open_fdv_lamports | u64 |
| min_open_fdv_lamports | u64 |
| max_open_fdv_lamports | u64 |
| fee_schedule | `{ cliff_fee_numerator u64, number_of_period u16, period_frequency u64, reduction_factor u64 }` |
| coins_launched | u64 |
| bump | u8 |
| reserved | [u8; 64] |

### Coin (2,290 bytes)

| offset | field | type |
|---|---|---|
| 8 | mint | Pubkey |
| 40 | launcher | Pubkey |
| 72 | bump, vault_bump | u8, u8 |
| 74 | state | u8: 0 digging, 1 ended, 2 opened |
| 75 | dig_minutes | u8 |
| | total_blocks | u16 |
| | launcher_fee_bps | u16 |
| | share_target | u32 |
| | open_fdv_lamports | u64 |
| | fee_schedule | 26 bytes |
| | launched_at, dig_start, dig_end | i64 × 3 |
| | current_block | u32 |
| | block_open_ts | i64 |
| | challenge | [u8; 32] |
| | target | u64 |
| | shares_this_block | u64 |
| | carry | u64 |
| | total_shares | u64 |
| | allocated | u64 |
| | claimed | u64 |
| | unmined | u64 |
| | opened_at | i64 |
| | pool, position, position_nft_mint | Pubkey × 3 |
| | pool_liquidity | u128 |
| | pool_sqrt_price | u128 |
| | pool_tokens | u64 |
| | collects | u64 |
| | fees_collected | u64 |
| | fees_to_launcher | u64 |
| | fees_to_treasury | u64 |
| | tokens_per_share | [u64; 90] |
| | block_shares | [u32; 90] |
| | block_targets | [u64; 90]: target each closed block was mined at. Expected hashes = shares × 2^64 / target (`blockHashes` in the SDK). |
| | miners_total | u64: lifetime count of distinct miners (miner accounts close at claim) |
| | token_2022 | bool: Token-2022 mint with metadata in the mint (false for coins launched before this field: classic + Metaplex) |
| | open_reserve_lamports | u64: what the launcher parked for `open` (0 before this field: a flat 0.035 SOL) |
| | reserve_refunded | u64: lamports `open` / `collect` returned to the launcher |
| | reserved | [u8; 47] |

The three new fields took 17 of the 64 reserved bytes, so the account is still 2,290 bytes and older coins decode unchanged (their bytes were zero).

### Miner (141 bytes)

| offset | field | type |
|---|---|---|
| 8 | coin | Pubkey |
| 40 | beneficiary | Pubkey |
| 72 | rent_payer | Pubkey |
| 104 | block | u32 (`0xFFFFFFFF` = none) |
| | block_shares | u64 |
| | next_nonce | u64 |
| | pending | u64 |
| | total_shares | u64 |
| | bump | u8 |

### Events

`CoinLaunched`, `SharesSubmitted` (beneficiary, block, accepted nonces, challenge, target), `BlockAdvanced` (new challenge + target), `CoinOpened`, `Claimed`, `FeesCollected`, `ReserveRefunded` (coin, launcher, amount). Discriminators are in `sdk/src/constants.ts` as `EVENT_DISC`; `parseEvents(logs)` in the SDK decodes them from transaction logs. Program error codes are in `ERRORS`.

For live mining, subscribe to the Coin account (`onAccountChange`) and read `current_block`, `challenge` and `target`.

## Invariants

- No token exists before `open`. Claims require `OPENED`, and `open` creates the pool in the same transaction that first mints anything.
- Supply is fixed by the rules:
  - `pool_tokens + burned = 800,000,000e6`
  - `allocated + unmined = 200,000,000e6`
  - `claimed ≤ allocated`
  - So total supply ≤ 1,000,000,000.
- A share only ever credits the beneficiary hashed into it. Within a block, nonces per beneficiary are strictly increasing, so a share can't be replayed.
- The challenge for block n is unknown until block n opens on chain (newest slot hash), so nothing can be precomputed.
- The pool position is owned by a PDA with no instruction to move or withdraw it, and it is permanently locked inside cp-amm.

## SDK quick use (`sdk/`)

The SDK is ESM. Build it with `npm run build`, which writes `dist/`. It imports three bare specifiers: `@solana/web3.js` (v1), `js-sha3` and `buffer`. A vanilla-JS site needs an import map for them, for example via esm.sh or jsdelivr `+esm`.

```js
import {
  coinAddresses, decodeCoin, submitSharesIx, mineRange, computeBudget, COMPUTE_UNITS, difficulty,
} from "./sdk/dist/index.js";

const { coin } = coinAddresses(mint);
const c = decodeCoin((await connection.getAccountInfo(coin)).data);
// scan nonces upward from the miner's next_nonce (0 at a new block)
const { found, nextNonce } = mineRange(c.challenge, wallet, c.target, 0n, 500_000, 8);
if (found.length) {
  const ixs = [
    ...computeBudget(COMPUTE_UNITS.submitShares),
    submitSharesIx({ payer: burner, mint, beneficiary: wallet, block: c.currentBlock, nonces: found }),
  ];
  // sign with the burner session key and send
}
```

What the rest of the SDK gives you:

- **PDAs:** `findConfig`, `findCoin`, `findVault`, `findMiner`, `findCpPool` and friends, or `coinAddresses(mint)` for all of them at once.
- **Instruction builders:** for every instruction listed above.
- **Decoders:** `decodeConfig`, `decodeCoin`, `decodeMiner`, plus `minerClaimable`.
- **Hashing:** `challengeHash`, `shareHash`, `shareValue`, `sharePreimage`.
- **Math:** `retarget`, `rewardForBlock`, `difficulty`, `expectedSharesPerSecond`, `sqrtPriceFromFdv`, `fdvSol`, `poolFeeAt`.
- **Events:** `parseEvents(logs)`, `decodeEvent(bytes)`.
- **Compute limits:** `COMPUTE_UNITS`, measured end to end (see `TESTING.md`).
- **Constants:** for the program and the Meteora accounts, and `ERRORS` (code → name).

## Build, test, deploy

```bash
# unit tests (host): share hash vs sdk vectors, retarget, rewards, dig state machine
CARGO_BUILD_JOBS=1 cargo test -j1 --lib

# program + IDL (target/deploy/first_nonce.so, target/idl/first_nonce.json)
CARGO_BUILD_JOBS=1 anchor build

# SDK
cd sdk && npm install && npm run build && npm test     # npm test checks dist/ against test-vectors.json

# end to end, in-process (LiteSVM + the real mainnet Meteora/Metaplex/Token programs); see TESTING.md
cd tests-e2e && npm install && npm run fetch && npm test
```

**Devnet:**

1. `solana config set --url devnet`. Fund the deploy wallet with ~3 SOL; the program is 388 KB (opt-level "z"), about 1.97 SOL of program-data rent at today's mainnet rent.
2. `anchor deploy --provider.cluster devnet`.
3. Call `init_config(admin, treasury)` from the upgrade-authority wallet (`initConfigIx` in the SDK).
4. Run a full cycle: launch a 5-minute coin → mine with the SDK's `mineRange` (or the browser miner) → `open` → swap on the pool → `collect` → `claim` → `seal`.

cp-amm and Token Metadata are deployed on devnet under the same ids.

**Mainnet:** deploy only after the devnet cycle passes and the code has been reviewed. Keep the upgrade authority on a hardware wallet or multisig.
