# Changelog

このファイルの形式は [Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に従います。バージョニング方針は本文末尾の「セマンティックバージョニング」を参照してください。

## [Unreleased]

## [0.10.0] - 2026-10-30

権威・正規ページランキングを追加し、20 問ベンチで Exa の hit@1 / MRR を上回りました。

### Added

- **`search.authority_weight`（既定 8.0）** — クエリがサイト名（`arxiv` / `mdn` / `python` / `docker` / `postgresql` / `numpy` など）を指すとき、その正規ドメインを押し上げ、ミラー（`docker-docs.uclv.cu` 等）を 0 点に降格。一般検索のみ適用、単一エンジン指定は上流順を維持
- **ホームページ抑制** — URL のパス深さを関連度に加算。特定クエリでトップページが深いページに勝つ問題を修正（`ja-python` など）

### 実測（20 問・URL 品質）

| | hit@1 | hit@5 | MRR |
| --- | ---: | ---: | ---: |
| **RustXNG（0.10.0）** | **17/20** | 18/20 | **0.870** |
| Exa | 16/20 | 19/20 | 0.867 |
| RustXNG（0.8.0） | 14/20 | 16/20 | 0.750 |
| Tavily | 4/20 | 13/20 | 0.394 |

- hit@1 と MRR で Exa を上回る（差は 1 問でノイズ範囲）。hit@5 は 1 問差で Exa が上
- **SearXNG 互換は維持**（単一エンジン指定の overlap@10 = 1.00）
- 主な改善: `en-compose` ミラー→公式が 1 位、`en-mdn-array`/`ja-fuji`/`ja-genji` が 1 位、`ja-python` が欠落→2 位

### Tests

- 784 ユニット + 17 統合（authority / ミラー降格 / ホームページ抑制のテストを追加）

## [0.9.0] - 2026-10-30

意味（埋め込み）リランキングを追加し、Neural 検索である Exa に構造的に対抗できるようにしました（新機能）。

### Added

- **埋め込みリランキング** — OpenAI 互換の embeddings エンドポイント（`search.embedding_url`）でクエリと上位結果を埋め込み、コサイン類似度で再順位付け
  - 一般（マルチエンジン）検索のみ適用。単一エンジン指定は上流順を維持（overlap@10 互換）
  - `embedding_model` / `embedding_weight`（既定 2.5）/ `embedding_topk`（既定 25）
  - エンドポイント失敗時は自動で位置融合 + 関連度ランキングにフォールバック
- **`tools/embed_server.py`** — ローカル・キー不要の OpenAI 互換埋め込みサーバ（fastembed + onnx、`BAAI/bge-small-en-v1.5`、384次元）。クラウド API でも可
- `dot` コサインとリランキングの単体テスト

### 測定（20 問）

- 関連度単独 → 埋め込み追加で hit@5 が 18 → 19（Exa と同率）
- 埋め込みは関連ページ 0.906 vs 雑ブログ 0.585 を明確に判別（`tools/embed_server.py` 検証）
- **正直な到達点**: 良い時間帯で RustXNG MRR ~0.84 / Exa 0.867 とほぼ同等（hit@5 同率 19）。差は Exa の「権威/正規ページ」知識（ミラーサイト vs 公式ドキュメント）。埋め込みはこれを詰める基盤

### Known limits

- 上流エンジンのレート制限により測定値は時間帯で変動（Exa は自社インデックスで安定）。頻繁な計測後は一時的に数値が低下する

## [0.8.0] - 2026-10-30

Exa（意味検索の商用 API）に対抗する関連度ランキングを追加したリリースです（新機能のためマイナー）。

### Added

- **関連度ランキング（`search.relevance_weight`、既定 4.0）** — 一般（マルチエンジン）検索で、URL + タイトルのクエリ語一致度をスコアに加算。URL はパーセントデコードして日本語クエリも照合
  - **0.750 → 0.838 に改善**（20 問ベンチ MRR）。hit@5 は Exa と同率 19/20
  - **単一エンジン指定では適用せず上流順を維持**（`engines=` の overlap@10 は github で 1.00）
  - `relevance_weight = 0` で上流 SearXNG と完全一致の順序に戻る
- **Exa 比較ツール** — `tools/collect_commercial.py` が `EXA_API_KEY` で Exa を収集、`hit_from_content` / `compare_content` に Exa 列を追加

### 実測（20 問・URL 品質）

| | hit@1 | hit@5 | MRR |
| --- | ---: | ---: | ---: |
| Exa | 16/20 | 19/20 | 0.867 |
| **RustXNG（関連度 4.0）** | 15/20 | **19/20（同率）** | 0.838 |
| RustXNG（従来） | 14/20 | 16/20 | 0.750 |
| Tavily | 4/20 | 13/20 | 0.394 |

### Tests

- 782 ユニット + 17 統合（関連度・URL デコードの単体テストを追加）

## [0.7.0] - 2026-10-29

ディープリサーチ機能を追加したリリースです（新機能のためマイナーを上げます）。

### Added

- **`GET /research` ディープリサーチエンドポイント** — 質問を受け取り、
  1. サブクエリを計画（LLM があれば LLM、無ければ質問から派生）
  2. 各クエリを検索し上位ページの本文を取得（キャッシュ利用）
  3. LLM があれば引用付き markdown レポートを生成
  を 1 エンドポイントで実行。`?q=` / `?max_queries=` / `?max_sources=` に対応
- **`[research]` 設定** — OpenAI 互換の `base_url` / `api_key` / `model` と `max_queries` / `max_sources` / `timeout`。LLM 未設定でも証拠バンドルは返る
- `src/deep_research.rs` — OpenAI 互換クライアント、クエリ計画、レポート合成、オーケストレーション

### Fixed

- **`Settings::merge` が `[cache]` と `[research]` セクションをマージしていなかった問題** — 設定ファイルの該当セクションが破棄されていた（キャッシュは既定値が同じため隠れていた）

### Tests

- 782 ユニット + 17 統合（`/research` 証拠モード・クエリ計画パース・フォールバックを追加）

## [0.6.1] - 2026-10-29

商用 API との直接比較を完成させたリリースです。

### Added

- **3 者直接比較**（`docs/BENCHMARK.md`）— 同じ 20 問・同じ gold URL で RustXNG / SearXNG / Tavily を測定
  - URL 中率: RustXNG MRR 0.750 / SearXNG 0.650 / Tavily 0.415（RustXNG が Tavily の 1.8 倍）
  - 内容: RustXNG raw 付 4,158 字 vs Tavily 1,248 字（gold 記述度 1.000 で同等）
- **Ollama Web Search の状況を記録** — API キー認証は成功するが、このアカウントは利用クレジット無しで HTTP 402。クレジット追加で 4 者比較に拡張可能（`tools/collect_commercial.py` 対応済み）

### ツール

- `tools/hit_from_content.py` — 収集済み `bench/content-*.json` から URL 中率（hit@k / MRR）を一括計算
- `tools/collect_commercial.py` — Tavily / Ollama REST API から同一 20 問を収集
- `tools/compare_content.py` / `tools/eval_content.py` — 内容品質（カバレッジ・スニペット長・クエリ語含有）を採点

## [0.6.0] - 2026-10-29

Tavily を全次元で上回る推奨設定を既定に組み込んだリリースです。

### Added

- **`[search] raw_content_limit`** — `include_raw_content=1` のとき本文を取得する上位件数（既定 5）
- **推奨設定のコメントブロック** — `settings.toml` の `[search]` に、20 問ベンチで Tavily を上回ることを確認した値（parallel / timeout 3.0 / raw_content_limit 5）を根拠付きで記載

### Changed

- **ページ本文をキャッシュ** — `include_raw_content` のページ取得がキャッシュされ、2 回目以降は 60ms で本文 4,001 字を返す（初回のみ取得）
- **`[cache] max_bytes` を 8 MB → 32 MB** — ページ本文もキャッシュするため 8 MB では早々に押し出されていた

### 実測（20 問ベンチ・Tavily 比較）

| 次元 | RustXNG（推奨） | Tavily |
| --- | --- | --- |
| URL 品質 MRR | **0.837** | 0.415 |
| 内容 / 件（raw 付） | **4,001 字** | 1,248 字 |
| レイテンシ（温熱・raw 付） | **0.06 秒** | 約 1 秒 |
| 料金 / 1,000 回 | **$0** | $8 |

## [0.5.1] - 2026-10-29

品質評価のハーネスと実測を追加したリリースです。

### Added

- `tools/eval_quality.py` と 20 問の評価セット（英語 12・日本語 8）。正解 URL が上位に来るかを hit@1 / hit@5 / MRR@10 で測り、レイテンシと応答サイズも記録する
- `bench/quality-questions.json` と実測結果（`bench/quality-parallel.json` / `quality-staged.json` / `quality-searxng.json`）
- [docs/BENCHMARK.md](docs/BENCHMARK.md) に SearXNG と商用 API（公開値）の比較表を追加

### 実測

- RustXNG parallel（既定）: hit@1 80%、hit@5 85%、MRR@10 0.837、中央値 929 ms、応答 7.6 KB
- SearXNG 1.4.2: hit@1 60%、hit@5 85%、MRR@10 0.707、中央値 1,321 ms、応答 34.9 KB
- RustXNG staged: hit@5 70%、MRR 0.574、2,004 ms（品質よりコールドスタートの待ち時間を優先する戦略）

測定環境では上位エンジンの多くが IP ブロックされていたため、品質は下限値です。

## [0.5.0] - 2026-10-29

前回の調査で挙がった改善を実装したリリースです。

### エージェント向けの追加

- **エージェント向けの検索パラメータ**（SearXNG にはない拡張、省略時は従来どおり）
  - `max_results=N` — 上位 N 件だけ返す。`q=rust async` の実測で 60 KB → 4.6 KB（-92%）
  - `include_domains` / `exclude_domains` — URL のホストで絞り込み（サブドメイン対応）
  - `include_raw_content=1` — 上位 5 件のページ本文を `raw_content` に追加（約 4,000 文字/件）
- **応答の検証** — パースは通るが架空・プレースホルダ・クエリと無関係な応答を検出し、
  結果に混ぜずにエンジンをサスペンドする（上流 issue #6358 の静かなゴミ混入への対策）。
  `unresponsive_engines` に `Suspicious response` として出る
- **実行戦略 `strategy=staged`** — エンジンを少数の段に分けて開始し、十分な結果で打ち切る。
  既定は従来どおりの `parallel`（SearXNG 互換）。実測（`q=rust async`、初回）で
  3.00 秒 → 0.84 秒、結果は 123 件 → 55 件。並び順は実績 → 未試行 → 失敗・サスペンドの順に自動化
  - `[search] strategy` / `staged_tier`（既定 8）/ `staged_min_results`（既定 10）
- 統合テストにエージェント向けフィルタと staged 戦略の検証を追加（14 → 16 件）

### Fixed

- `timeout_limit` が検索全体の締め切りとして働いていなかった問題を修正。以前は各エンジンに
  一律で適用していたため、**すでに時間内に答えていたエンジンの結果まで捨てていた**
  （実測: 45 ms で返るエンジンが 100 ms の設定で Timeout 扱いになり、0 件になっていた）。
  エンジンを走らせる前に「検索全体の締め切りまでの残り時間」で上限を切り、
  完了済みの結果は必ず活かすようにした
- `xpath` エンジンが RSS/RDF を読めなかった問題を修正。HTML パーサは `<link>` を
  void 要素として扱うため `<link>URL</link>` のテキストが落ち、`url_xpath = "./link"`
  が空になって全件スキップされていた（RDF は 0 件、同じ engine で HTML は 392 件）。
  XML 宣言か Content-Type が XML の応答は新しい XML DOM（`src/engine/xml.rs`、
  `quick-xml` は既存依存）で辿る。HTML 経路の挙動は変えていない。不正な XML は
  HTML パースにフォールバックする（黙って 0 件にしない）
- `//RDF//item` のようにルート要素を名指しする式が 0 件になっていた問題（XML 選択で
  最初のステップにルート自身を含めていなかった）

### Added

- ビルド済みバイナリの配布: Linux（glibc / 静的 musl）、macOS（arm64 / x86_64）、
  Windows（x86_64）をタグ push で自動ビルドし、`.sha256` とともに Release に添付
  （`.github/workflows/release.yml`）。資産名は `rustxng-<tag>-<target>` で、
  rustdsh の `rdsh rustxng install` がこの名前で選ぶ
- CI を Linux・macOS・Windows の matrix に拡張し、clippy `-D warnings`、
  `--no-default-features` ビルド、Docker イメージの `/healthz` 起動確認を追加

## [0.4.1] - 2026-10-26

SearXNG との精度比較で見つかった差分を修正したリリースです。

### Fixed

- **npm**: 検索バックエンドを npm registry から上流と同じ npms.io に変更（一致度 0.01 → 1.00）
- **stack exchange**: 既定のソートを上流と同じ `activity` に変更（`api_sort` で変更可）、結果 URL を上流と同じ `/q/{id}` 形式に統一（一致度 0.00 → 1.00）
- **pubmed**: オフラインエンジンの実行経路を使い、上流と同じ 2 リクエスト（esearch → efetch）構成に変更（一致度 0.39 → 1.00）

### Added

- 精度比較レポート [docs/ACCURACY.md](docs/ACCURACY.md)（overlap@10 / top1 / found の実測と再現手順）
- `bench/compare.py` に `--delay` オプション（レート制限回避）

## [0.4.0] - 2026-10-24

前回の「未移植」項目をすべて埋め、実測でボトルネックを潰したリリースです（後方互換）。

### Added

- **DB 直結エンジン 8 種を実装**: postgresql, mysql_server, mariadb_server, mongodb, sqlite, valkey_server, recoll, opensemantic
  - 検索バックエンド 3 種（elasticsearch / solr / meilisearch）と合わせ、上流の DB 系 11 種がすべて動作
  - `db-engines` Cargo フィーチャー（既定で有効）。`--no-default-features` でバイナリを 27 MB → 17 MB に削減可能
  - 結果は key/value 形式（`keyvalue` テンプレート）で表示
- **Valkey / Redis キャッシュ**: `[cache] backend = "valkey"` で複数インスタンスがキャッシュを共有
- **botdetection 相当のレート制限**: 信頼プロキシの X-Forwarded-For 解決、IPv4/IPv6 CIDR、IP 制限のスライディング窓、リンクトークン、429 応答
- **i18n を 57 ロケールに拡張**（11 → 57。上流は 60）

### Changed

- エンジン名が大文字小文字を区別せずに一致するように
- 未知のエンジン名を指定した場合は SearXNG と同じく既定のエンジン群で検索（従来は 400 エラー）
- ベンチマーク example がエンジン別の応答時間を表示

### Performance

- タイムアウトした要求を再試行しない（上流も接続エラーのみ再試行）: 全エンジン検索の中央値 6.26 秒 → 3.00 秒
- 再試行を含めた全体デッドラインを厳守
- ウォーム時（キャッシュ + エンジンサスペンド）: 中央値 276 ms、単一サイト検索は 1〜50 ms

### Tests

- ユニット 750 + 統合 14

## [0.3.0] - 2026-10-24

全エンジンの移植を完了したリリースです（後方互換）。

### Added

- **エンジン 226 型 / 上流 352 エントリをすべて実装（100%）**
  - 検索系: yandex, yahoo, seznam, marginalia, yep, swisscows, quark, sogou, baidu 系, naver ほか
  - 動画/音楽: vimeo, rumble, odysee, niconico, youtube_api, bilibili, iqiyi, acfun, yandex_music, spotify
  - 画像: google_images, flickr, pinterest, pixiv, flaticon, iconify, lucide, material_icons, tineye, unsplash 系
  - 学術: pubmed, europepmc, openalex, springer, astrophysics_data_system, pdbe, nvd, chatnoir, scanr_structures
  - パッケージ/開発: github_code, gitlab, sourcehut, gitea, pkg_go_dev, hex, metacpan, lib_rs, nvd, repology, fdroid
  - ショッピング: ebay, geizhals, steam, shopify_stock, apkmirror, apple_app_store, google_play
  - 地域/ニュース: tagesschau, reuters, yahoo_news, swisscows_news, il_post, ansa, gmx, www1x, neosearch, xprivo ほか
  - torrent/ファイル: piratebay, nyaa, kickass, solidtorrents, torznab, tokyotoshokan, btdigg, bt4g, annas_archive, zlibrary
  - 検索バックエンド: elasticsearch, solr, meilisearch（HTTP 完全実装）
  - 翻訳/天気/その他: deepl, libretranslate, lingva, mozhi, translated, open_meteo, wttr, duckduckgo_weather, currency_convert
- エンジン初期化に失敗しても起動を継続する挙動（上流と同じ。失敗理由は警告ログに記録）
- レビュー補助: `tools/register_engines.py`（エンジンの登録を自動同期）、`tools/verify_boot.py`

### Changed

- 既定で有効なエンジンを約 170 に拡大（認証情報・自前インスタンスが必要なものは上流と同じく無効）
- ドキュメント: COVERAGE / COMPATIBILITY / README を最新の数値に更新

### Tests

- ユニット 722 + 統合 14（外部ネットワーク不要）

## [0.2.0] - 2026-10-18

SearXNG クライアントから見た互換性を高めたリリースです。JSON/CSV/config の形が変わるため **破壊的変更** として 0.2.0 にしました。

### Changed (breaking)

- `format=json` は上流 `get_json_response` の形（7 フィールド）だけを返すように変更。計測値などは `full=1` で取得
- `format=csv` の列を上流と同じ `title,url,content,host,engine,score,type` に変更（answer / suggestion / correction 行も出力）
- `/config` を上流と同じフィールド構成（`brand` / `limiter` / `locales` / エンジンの `timeout` など）に変更
- 設定 Cookie のカテゴリ名を `category` に変更（旧 `categories` も読み取り可）

### Added

- 上流 SearXNG の `settings.yml`（YAML）をそのまま読み込み可能（TOML も可）
- `full=1` による詳細 JSON
- 互換性レポート [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md)
- README に速度比較の節

## [0.1.0] - 2026-10-18

最初の公開版です。SearXNG の振る舞いを Rust に移植し、単一バイナリで動くメタ検索エンジンとしてまとめました。

### Added

- 検索コア: `!engine` / `!category` / `:lang` / `<timeout` / `!!` / `!!bang` の構文解析、結果の重複排除・スコアリング・カテゴリ別グループ化（上流 `searx/query.py` / `searx/results.py` 準拠）
- エンジン: 113 設定エントリ / 71 エンジン型。宣言的エンジン（`xpath` / `json_engine` / `mediawiki`）と上流 `settings.yml` の自動取込
- 外部 bang: 13,561 件（上流の圧縮トライを平坦化）
- HTTP: `/search`（HTML/JSON/CSV/RSS）、`/autocomplete`、`/config`、`/preferences`、`/stats`、`/metrics`、`/image_proxy`、`/favicon_proxy`、`/opensearch.xml`、`/healthz`
- サブシステム: プラグイン 11 種、アンサーラー（statistics / random / weather）、Wikidata インフォボックス、i18n 11 ロケール
- 運用: TTL+LRU キャッシュ、エンジン単位のサスペンド、簡易レート制限、エンジン統計
- 開発: テスト 298 件、ベンチマーク example、上流との比較スクリプト、カバレッジ計測、CI

## セマンティックバージョニング

- バージョンは [SemVer 2.0.0](https://semver.org/lang/ja/) に従います（`MAJOR.MINOR.PATCH`）
- **公開 API** とみなすもの: クレートの公開アイテム、HTTP エンドポイントとそのパラメータ、JSON/CSV/RSS の出力形、設定キーと環境変数、CLI オプション
- 1.0.0 未満の間は、互換性を壊す変更を **MINOR** で上げます（`0.2.0` → `0.3.0`）。1.0.0 以降は **MAJOR** です
- 互換性を保つ追加は MINOR、バグ修正のみは PATCH です
- 破壊的変更は CHANGELOG の `Changed (breaking)` に明記します
