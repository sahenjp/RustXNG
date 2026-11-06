# SearXNG 互換性の範囲

**RustXNGはSearXNGの主要なHTTP API・設定形式・検索構文との互換性を目指していますが、完全な動作互換を保証するものではありません。** 352エントリ/226型は過去の移植時点における型の実装数であり、現在の既定検索元の数や、全エンジンの応答・利用許諾を検証した数字ではありません。[移植範囲と履歴](COVERAGE.md)・[検索元の監査](PROVIDERS.md)を分けて確認してください。

## 互換と言える範囲

| 領域 | 状態 | 根拠 |
| :--- | :--- | :--- |
| 検索構文（`!engine` / `!category` / `:lang` / `<timeout` / `!!` / `!!bang`） | **互換** | 上流 `searx/query.py` と同じ判定順・同じ正規化 |
| 結果の統合（重複排除・スコア・カテゴリ別グループ化） | **互換** | `searx/results.py` の `calculate_score` / `get_ordered_results` と同じ式 |
| `/search` のパラメータ名・出力形式（html/json/csv/rss） | **互換** | 統合テストで検証 |
| JSON API の形 | **互換（既定）** | 上流 `get_json_response` の 7 フィールドのみを出力。詳細版は `full=1` |
| JSON の各要素の形 | **互換** | `answers` = `[{"answer":...}]`、`unresponsive_engines` = `[[engine, "Translated error"]]`、`template` = `"default.html"`、`metadata` = 文字列、`priority` 既定は空文字 |
| CSV 出力 | **互換** | 上流 `write_csv_response` と同じ列（`title,url,content,host,engine,score,type`）と同じ行種別（result / answer / suggestion / correction） |
| `/config` | **互換** | 上流と同じフィールド（`brand`、`limiter`、`locales`、`engines[].timeout` などを含む） |
| 設定ファイル | **互換（YAML をそのまま読める）** | `--config settings.yml` / `SEARXNG_SETTINGS_PATH` に上流の YAML をそのまま指定可能。TOML も可 |
| 環境変数（`SEARXNG_*`） | **互換** | 同じ名前を受け付け |
| 外部 bang | **データ互換** | 上流の 13,561 件を同一データから生成 |
| Cookie 名 | **互換** | `category` / `language` / `locale` / `safesearch` / `theme` / `disabled_engines` など上流と同じ（旧名 `categories` も読み取り可能） |
| OpenMetrics のエンジン指標名 | **互換** | `searxng_engines_*` |

## JSON API

既定の `format=json` は上流の形だけを返します。

`length` / `iframe_src` / `audio_src` / `views` など本実装が持たないフィールドは、エンジン側の `metadata` 文字列に含まれます（上流と同じ「メタデータ」表現）。

計測値を含む詳細版が必要な場合は `format=json&full=1` を使います（`number_of_results` / `timings` / `engines` / `engine_data` / `paging` / `redirect_url` / `error` が追加されます）。

## エージェント向けの拡張（上流にないパラメータ）

SearXNG のパラメータと衝突しない名前で、LLM のツール呼び出し向けの指定を追加しています。省略すれば従来の挙動のままです。

| パラメータ | 意味 | 返り値への影響 |
| :--- | :--- | :--- |
| `max_results=N` | 並べ替え後の上位 N 件だけを返す（1〜200 に丸める） | `results` を切り詰め |
| `include_domains=a.com,b.com` | URL のホストが指定ドメイン（サブドメイン含む）の結果だけを残す | 絞り込み |
| `exclude_domains=a.com` | 指定ドメインの結果を落とす | 絞り込み |
| `include_raw_content=1` | 上位 `search.raw_content_limit` 件（既定 5）のページ本文を取得して `raw_content` フィールドに入れる | `search.raw_content_chars` 文字/件まで（既定 8,000、変更可能）。打切り時は `…` と `raw_content_truncated` を追加 |
| `accept_language=ko-KR,ko;q=0.9,en;q=0.8` | `include_raw_content` で取得するページの言語優先度を明示する | 指定を `Accept-Language` として送信し、言語ごとに本文キャッシュを分離。検索結果の言語指定とは独立 |
| `strategy=staged` | エンジンを少数の段に分けて開始し、`search.staged_min_results` 件で打ち切る（既定は並列） | 応答時間は短縮、返る件数は減る |

**ランキングの差分（`search.relevance_weight`）**: 一般（マルチエンジン）検索では、URLとタイトルのクエリ語一致度でスコアを補正します（既定4.0）。単一エンジン指定ではこの補正をせず、取得元の並びを維持します。`relevance_weight = 0`でこの補正を無効化できますが、検索先・統合・フィルタの違いがあるため、SearXNGの結果順全体が同一になる保証ではありません。

応答自体の検証も行います。パースは通るが内容が明らかに架空・プレースホルダ・クエリと無関係な応答（上流 issue #6358 の「静かなゴミ混入」）は、結果に混ぜずにエンジンをサスペンドし、`unresponsive_engines` に `Suspicious response` として出します。

### ページ本文の言語

MCP `read_page` の `accept_language` と HTTP `/search` の同名パラメータは、同じ検証と本文取得処理を使います。HTTP では次のように指定します。

```bash
curl --fail --get 'http://127.0.0.1:29483/search' \
  --data-urlencode 'q=서울 날씨' --data 'format=json' \
  --data 'include_raw_content=1' \
  --data-urlencode 'accept_language=ko-KR,ko;q=0.9,en;q=0.8'
```

省略または空文字列ならヘッダを送りません。検索の `language`・`locale`、サーバーのロケール、HTTP クライアントから受けたヘッダからは推測しません。
値は最大512 ASCIIバイト、16言語範囲です。[RFC 4647 §2.1](https://www.rfc-editor.org/rfc/rfc4647.html#section-2.1) の基本言語範囲（`ko-KR`、`zh-Hant`、`*` など）と、[RFC 9110 §12.4.2](https://www.rfc-editor.org/rfc/rfc9110.html#section-12.4.2) の重み（`q=0`〜`q=1`、小数点以下3桁まで）を受け付けます。改行や不正な値は MCP エラー、HTTP 400 になり、ページ取得を開始しません。

大文字小文字・区切りの空白・重みの末尾のゼロは正規化し、指定順は保ちます。本文キャッシュは URL・抽出文字数上限・正規化した指定値で分離され、言語未指定の本文も別に扱います。MCP で `next_offset` を使う際は、同じ `accept_language` を続けて渡してください。キャッシュは引き続き最大8件で、HTTP の有効期限も変わりません。

指定は直接取得するページとそのリダイレクト、JavaScript レンダラーが RustXNG 経由で取得する GET/HEAD リソースに適用します。リソースは別オリジンの場合も含みますが、宛先検証と取得制限はすべて維持します。`robots.txt` は言語指定なしで取得する従来のポリシーを保ちます。レンダラーへ Cookie や任意のヘッダを渡す機能ではなく、ブラウザー内の `navigator.language` も変更しません。

`fetch.provider = "direct"` で利用できます。[DeepSeek Fetch の公開スキーマ](https://api-docs.deepseek.com/api/fetch/)には取得先の言語指定がないため、外部プロバイダーでの明示指定は取得エラーになります。MCP はエラーを返し、HTTP は他の取得失敗と同様に `raw_content` を付けません。指定を黙って無視したり、別の取得先へ切り替えたりはしません。言語指定は翻訳ではなく、サイトがその言語を返す保証もありません。403 後に別の言語で再試行する処理は追加していません。

### ディープリサーチ（`GET /research`、上流にない）

質問を受け取り、(1) サブクエリを計画（LLM があれば LLM、無ければ質問から派生）、(2) 各クエリを検索して上位ページの本文を取得、(3) LLM があれば引用付き markdown レポートを生成して返します。LLM 未設定でも証拠バンドル（`queries` / `sources` / 本文）は返ります。設定は `settings.toml` の `[research]`（`enabled` / `base_url` / `api_key` / `model` / `max_queries` / `max_sources`）。

## 互換でない領域

| 領域 | 差分 |
| :--- | :--- |
| **エンジンの網羅率** | 移植時の記録では上流352エントリ/226型の型対応を報告。**現行の同梱設定は273エントリ中34件が既定有効**。型の存在は実際の動作・許諾・結果一致を保証しない。内訳は [COVERAGE.md](COVERAGE.md) と [PROVIDERS.md](PROVIDERS.md) |
| **UI** | 単一テーマ（simple 相当）のみ。上流のテンプレート・テーマ資産とマークアップは非互換 |
| **i18n** | 57 ロケール（上流は 60） |
| **`/stats`** | 表示形式が上流と異なる（HTML の内容・`/stats/errors` の形） |
| **サブシステム** | プラグイン 11 / 上流 12、favicon は上流の resolver 方式ではない |
| **キャッシュ** | メモリ内（TTL + LRU）と Valkey / Redis の両方に対応 |
| **botdetection** | 信頼プロキシの XFF 解決、IP 制限のスライディング窓、リンクトークン、429 応答 |
| **DB 直結エンジン** | 11 種すべて実装済み（db-engines フィーチャー、既定で有効） |

## 検証の方法

- 構文・統合ロジック・JSON/CSV の形: `cargo test --locked --all-targets`。テスト数は変更により変わるため、現行CIの結果を確認してください。
- 上流との実測比較（速度・結果一致度）: [BENCHMARK.md](BENCHMARK.md) と `bench/compare.py`
- エンジン網羅率: `python3 tools/coverage.py /path/to/searxng`

## 完全互換に近づけるには

1. 過去の型対応記録（226/226）と実運用での互換性を分けて検証する。DB直結やHTTP経由の検索バックエンドには動作実装があり、すべてスタブという古い説明は現在の実装に合いません。
2. `/stats` と UI テンプレートを上流資産に合わせる
3. i18n のテンプレート全面適用とロケール拡大
4. UI テンプレートのマークアップを上流に合わせる

## 商用 API をエンジンとして併用する

RustXNGには商用検索APIを検索元として追加できます。**設定に登録されている型の数と、利用可能・許諾確認済みの検索元の数は別です。** APIの料金、クォータ、検索結果の利用許諾と再配布条件を確認してください。

`exaapi` / `braveapi` は実装済みで既定は無効です。設定ファイルで有効化します。

```toml
# my-settings.toml
use_default_settings = true

[[engines]]
name = "exaapi"
engine = "exaapi"
shortcut = "exa"
categories = ["general", "web"]
disabled = false          # 既定で無効なエンジンを有効化できる
api_key = "YOUR_EXA_KEY"
content_mode = "highlights"   # highlights | text
```

```bash
SEARXNG_SETTINGS_PATH=my-settings.toml ./rustxng --port 29483
```

> 0.13.0 で `EngineConfig::disabled` を三状態（`Option<bool>`）にしました。以前は `disabled = false` が「未指定」と区別できず、既定で無効なエンジンを有効化できませんでした。

### 過去の実測（100問・現在の構成ではない）

次の数値は旧構成で保存した観測であり、v0.30.0の再測定や同条件の最新Exa比較ではありません。開発に用いた質問集の結果だけで一般的な優劣を結論付けないでください。

| 構成 | hit@1 | hit@5 | MRR |
| --- | ---: | ---: | ---: |
| Exa 単体（API 直） | 74 | **88** | **0.808** |
| **RustXNG + exaapi** | **75** | 85 | 0.799 |
| RustXNG 単体（4 回平均） | 62 | 79 | 0.710 |

この旧測定ではhit@1が75対74、hit@5が85対88、MRRが0.799対0.808でした。**この1問の差を「RustXNGはExaより高精度」という一般的な主張には使えません。** 現行構成の品質は独立した質問集・同時刻の再測定で確認してください。
