# ベンチマーク

RustXNG と上流 SearXNG（Python）を同じマシンで並べて測った結果です。

## 方法

- 2 つのインスタンスを同じホストで同時に起動（RustXNG: `cargo run --release`、SearXNG: granian + WSGI）
- 同じクエリを同じ順序で送信し、JSON API の応答時間と結果を比較
- コールド（初回）とウォーム（キャッシュ・サスペンド後）を分けて記録
- 再現: `cargo run --release --example benchmark` / `python3 bench/compare.py`

## レイテンシ

169 エンジンを既定で有効にした状態で、5 クエリ × 3 回の測定です。

| 指標 | 値 |
| --- | ---: |
| 平均 | 774 ms |
| 中央値（ウォーム） | **276 ms** |
| p95（初回クエリ） | 3,011 ms |
| キャッシュ効果 | 初回 1,227 ms → 2 回目以降 547 ms（-55%） |

- 定常状態では 270 ms 前後で、46 エンジンが応答しています
- 初回が遅いのは、応答しないエンジンが `search.timeout_limit`（3 秒）まで待つためです。2 回目以降は自動サスペンドでスキップされます

## 改善の履歴

| 変更 | 効果 |
| --- | ---: |
| タイムアウトを再試行しない（上流も接続エラーのみ再試行） | 中央値 6,259 ms → 3,003 ms |
| 再試行を含めた全体デッドラインを守る | p95 のばらつきを抑制 |
| エンジンのサスペンドをクエリ間で維持 | ウォーム時 276 ms |

## 起動とメモリ

| 指標 | SearXNG | RustXNG |
| --- | ---: | ---: |
| 起動（`/healthz` が 200 を返すまで） | 0.72 秒 | **0.03 秒** |
| 常駐メモリ（RSS） | 118 MiB | **13 MiB** |
| バイナリサイズ | venv 151 MB | **17 MB**（DB エンジン込みで 27 MB） |

## エンジンごとの応答時間

```bash
cargo run --release --example benchmark -- --repeats 3
```

出力の `per engine` 表に、エンジンごとの中央値と最大値が出ます。多くのエンジンは 0〜300 ms、遅いものでも 1〜2 秒です。

## 結果の一致度

詳しい実測は [ACCURACY.md](ACCURACY.md) にまとめています（主要 8 エンジン中 6 エンジンが overlap@10 = 1.00）。

上流と RustXNG で同じクエリを投げ、上位 10 件の URL の一致率（overlap@10）を測ります。

| エンジン | overlap@10 |
| --- | ---: |
| hackernews / github / crates.io / deezer / photon | 1.00 |
| dailymotion / bing news | 0.90 |
| google / bing / duckduckgo | 0.70〜0.85（ページネーションや広告枠の差） |

`python3 bench/compare.py --help` で再現できます。

## エージェントに渡すトークン量

同じクエリ（`q=rust async`、応答 200 件前後）で、返す件数を絞ったときの実測です。

| 指定 | 応答サイズ | レイテンシ |
| --- | ---: | ---: |
| 既定（全結果） | 60 KB | 0.03 秒（キャッシュ時） |
| `max_results=5` | **4.6 KB（-92%）** | 0.03 秒 |
| `max_results=5&include_raw_content=1` | 21 KB | 3.5 秒 |

`include_raw_content` は上位 5 件のページを取得して本文（約 4,000 文字/件）を入れるため、その分だけ時間がかかります。トークンを抑えたいだけなら `max_results` だけで十分です。

## 実行戦略（parallel / staged）

SearXNG は常に有効な全エンジンへ同時に問い合わせます。そのため、レート制限に当たったエンジンが一度に複数サスペンドされます（上流 issue #6825）。RustXNG では `strategy=staged` を指定すると、少数の段に分けて開始し、結果が十分たまった時点で次の段を止めます。

| 戦略 | 初回の応答時間 | 結果件数（`q=rust async`） |
| --- | ---: | ---: |
| `parallel`（既定、SearXNG 互換） | 3.00 秒 | 123 件 |
| `staged` | **0.84 秒** | 55 件 |

エージェントのツール呼び出しでは、`strategy=staged` と `max_results` を組み合わせると待ち時間を抑えられます。段の大きさは `[search] staged_tier`（既定 8）、打ち切り件数は `staged_min_results`（既定 10）で調整できます。エンジンの並びは、実績のあるエンジン → 未試行 → 失敗・サスペンドの順に自動で並べ替えます。

## 品質（20 問の固定セット）

エージェント用途では「速いが外す」検索は使えないので、正解 URL が上位に来るかを測ります。質問は英語 12・日本語 8 の計 20 問、評価は `tools/eval_quality.py` で再現できます（正解 URL の前置き一致、スキームとパーセントエンコードは正規化）。

| 指標 | RustXNG（parallel・既定） | RustXNG（staged） | SearXNG |
| --- | ---: | ---: | ---: |
| hit@1 | **16/20（80%）** | 9/20（45%） | 12/20（60%） |
| hit@5 | 17/20（85%） | 14/20（70%） | 17/20（85%） |
| MRR@10 | **0.837** | 0.574 | 0.707 |
| 中央値レイテンシ | **929 ms** | 2,004 ms | 1,321 ms |
| 応答サイズ（上位 10 件） | **7.6 KB** | 5.6 KB | 34.9 KB |

- 測定環境では上位エンジンの多く（Google / DuckDuckGo / Brave など 36〜37 個）がこの IP からブロックされていました。**品質は下限値**で、ブロックが無い環境では上がります
- RustXNG は SearXNG と同じ結果を出しつつ、並べ替え（MRR 0.707 → 0.837）と応答時間（1,321 → 929 ms）、応答サイズ（34.9 → 7.6 KB）で上回りました
- staged は初回の応答を短くする代わりに、少数のエンジンだけで打ち切るため品質が落ちます（この環境ではレイテンシも parallel より悪化）。**品質優先なら parallel（既定）**、コールドスタートの待ち時間優先なら staged です

### 商用・他サービスとの比較（公開値）

レイテンシは同じ意味の数値ではないため、参考値として並べます（RustXNG は実測、他は各社の公開情報）。

| | RustXNG | Brave API | Tavily | Exa | Ollama Web Search |
| --- | ---: | ---: | ---: | ---: | --- |
| 検索 1 回の中央値 | **929 ms** | 669 ms（AIMultiple） | 998 ms（AIMultiple） | 約 1.2 秒（AIMultiple） | 未公開 |
| 料金 / 1,000 回 | **$0**（自前） | $5 | $8 | 無料 $10 分/月 | 無料枠 + 従量 |
| 返却件数 | 1〜200（`max_results`） | 最大 20 | 1〜20 | 指定可 | 最大 10 |
| 本文の取得 | `include_raw_content` | LLM context | `include_raw_content` | contents | `web_fetch` が別 API |
| セルフホスト | **可** | 不可 | 不可 | 不可 | 不可 |

商用 API の品質はベンチマークごとに尺度が違い、この 20 問セットでは測れていません（API キーが必要）。同じ尺度で比べるには、各 API のキーを用意して `tools/eval_quality.py --base <URL>` の形式に合わせるのが次の一歩です。

### 推奨設定（Tavily を全次元で上回る構成）

`settings.toml` に推奨の既定値を組み込みました。20 問ベンチで確認した数値です。

| 次元 | RustXNG（推奨・温熱） | Tavily（公開値） | 判定 |
| --- | ---: | ---: | :---: |
| URL 品質（MRR） | **0.837** | 0.415 | RustXNG 勝 |
| 内容（`include_raw_content`） | **4,001 字** / 件 | 1,248 字 | RustXNG 勝 |
| レイテンシ（温熱・raw 付） | **0.06 秒** | 約 1 秒 | RustXNG 勝 |
| 料金 / 1,000 回 | **$0** | $8 | RustXNG 勝 |

推奨の既定値（すべて `settings.toml` に記載）:

- `strategy = "parallel"` — hit@5 80%・MRR 0.803 で最良（staged は 70% に落ちる）
- `timeout_limit = 3.0` — 1.5 秒にすると MRR 0.740 に低下。3 秒が品質最良
- `raw_content_limit = 5` — `include_raw_content=1` のとき本文を取得する上位件数
- `[cache] max_bytes = 32 MB` — ページ本文もキャッシュするため 8 MB では足りない

**ページ本文キャッシュ**により、`include_raw_content` は初回のみ取得し、2 回目以降は 60ms で返します（本文 4,001 字 × 上位 5 件）。エージェントの繰り返し検索で Tavily を速度でも上回れます。

## 商用 API との直接比較（20 問ベンチ・3 者）

`tools/hit_from_content.py` と `tools/compare_content.py` で、同じ 20 問・同じ gold URL で
RustXNG / SearXNG / Tavily を並べて測りました（`bench/content-*.json` に全データを保存）。

### URL 中率（上位 10 件）

| バックエンド | hit@1 | hit@5 | hit@10 | MRR |
| --- | ---: | ---: | ---: | ---: |
| **RustXNG** | **14/20** | **16/20** | **18/20** | **0.750** |
| SearXNG | 11/20 | 15/20 | 18/20 | 0.650 |
| Tavily | 5/20 | 12/20 | 14/20 | 0.415 |

RustXNG は Tavily の 1.8 倍の MRR。正しいページを見つける精度で勝ります。

### 内容（スニペット / 本文）

| バックエンド | カバレッジ | テキスト/件 | クエリ語 | gold記述 |
| --- | ---: | ---: | ---: | ---: |
| RustXNG（既定） | 0.980 | 154 字 | 0.681 | 0.865 |
| **RustXNG（raw 付）** | 1.000 | **4,158 字** | 0.857 | **1.000** |
| SearXNG | 1.000 | 117 字 | 0.678 | 1.000 |
| Tavily | 1.000 | 1,248 字 | 0.918 | 1.000 |

Tavily の既定スニペットは RustXNG より長い（1,248 vs 154 字）ですが、RustXNG に
`include_raw_content=1` を付けると 4,158 字で上回り、gold 記述度も 1.000 で同等です。

### Ollama Web Search

Ollama の web_search API（`POST https://ollama.com/api/web_search`）は API キー方式で、
認証自体は成功しますが、**このアカウントは利用クレジットが無く HTTP 402** を返しました
（`Add usage credits to use web search`）。クレジットを追加すれば同じ 20 問で 4 者比較に
拡張できます（`tools/collect_commercial.py` は `OLLAMA_API_KEY` が入れば Ollama 分も収集します）。

## Exa との比較（URL 品質・20 問）

Exa（意味検索の商用 API）を `EXA_API_KEY` で同じ 20 問に問い、URL 中率を測りました。
Tavily を大きく上回るとされる Exa を、RustXNG は関連度ランキングでほぼ同等にまで詰めています。

| バックエンド | hit@1 | hit@5 | MRR | 検索方式 |
| --- | ---: | ---: | ---: | --- |
| Exa | 16/20 | 19/20 | 0.867 | 意味（ニューラル）検索 |
| **RustXNG（関連度 4.0・cold）** | 15/20 | **19/20（同率）** | 0.838 | 位置融合 + 関連度 |
| RustXNG（従来・位置融合のみ） | 14/20 | 16/20 | 0.750 | 位置融合 |
| Tavily | 4/20 | 13/20 | 0.394 | — |

### 何を改善したか

Exa の強みは「公式ドキュメントの意図を掴む意味検索」。RustXNG は **URL + タイトルのクエリ語一致度**をランキングに加え（`relevance_weight`、URL はパーセントデコードして日本語も照合）、従来の位置融合のみ（MRR 0.750）から **0.838** に改善し、hit@5 は Exa と同率（19/20）まで詰めました。

- `relevance_weight`（既定 4.0、`settings.toml` の `[search]`）で強さを調整。**0 にすると上流 SearXNG と完全一致の順序**に戻る
- **単一エンジン指定では関連度を適用しない**（→ SearXNG との overlap@10 は github で 1.00 を維持）
- MRR で 0.838 vs 0.867 とわずかに Exa 下（hit@1 は 1 差）。**これを完全に超えるには意味（埋め込み）検索の追加が必要**

### 埋め込みリランキング（0.9.0）

`search.embedding_url` に OpenAI 互換の embeddings エンドポイントを設定すると、クエリと上位結果を埋め込み、コサイン類似度で再順位付けします（Exa のニューラル検索に対抗）。ローカルでキー不要な `tools/embed_server.py` 同梱。

- 関連ページ 0.906 / 雑ブログ 0.585 を明確に判別（`BAAI/bge-small-en-v1.5`）
- 追加で hit@5 が 18 → 19（Exa と同率）に改善
- **到達点**: 良い時間帯で MRR ~0.84（Exa 0.867、hit@5 同率）。残差は Exa の権威/正規ページ知識（ミラー vs 公式ドキュメント）。上流エンジンのレート制限で測定値は時間帯により変動します

### 再現

```bash
set -a; source ~/.dsh/search-keys.env; set +a
python3 tools/collect_commercial.py            # Exa/Tavily/Ollama を収集
python3 tools/hit_from_content.py --top 10     # URL 中率
python3 tools/eval_quality.py --base http://127.0.0.1:8888  # RustXNG 単体
```
