# AIエージェント向け統合検索エンジン RustXNG

> 検索 API を 1 本に。複数のサイトを、自分のサーバーから。

## これは何か

**RustXNG はAI エージェントに検索させるためのセルフホスト型メタ検索エンジンです。**

Bing・DuckDuckGo・Wikipedia・GitHub・arXiv・PubMed など複数のサイトを 1 つの JSON API にまとめ、Rust の単一バイナリで動きます。Python もデータベースも API キーも要りません。

SearXNG の API・検索構文・設定形式をそのまま引き継いでいるので、既存の SearXNG 前提のツールやプロンプトはそのまま動きます。

## なぜ AI エージェントに良いのか

### 1. ツール呼び出しに耐える速さ

| 指標 | 数値 |
| :--- | ---: |
| 起動 | 0.03 秒 |
| 常駐メモリ | 13 MiB |
| 単一サイト検索（ウォーム） | **1 ms** |
| 全 169 サイト検索（ウォーム中央値） | **276 ms** |

エージェントは 1 ターンで何度も検索します。1 回 1 秒の API を 5 回呼ぶとそれだけで 5 秒。RustXNG はキャッシュヒット時 1 ms 台なので、ツール呼び出しの待ち時間がほぼ消えます。

### 2. スキーマが固定で壊れない

レスポンスは SearXNG 互換の JSON（`query` / `results` / `answers` / `infoboxes` / `suggestions` / `unresponsive_engines`）。LLM に渡す形が毎回変わらないので、プロンプトとパーサが安定します。

```json
{
  "query": "rust async runtime",
  "results": [
    {
      "url": "https://tokio.rs/",
      "title": "Tokio — an asynchronous Rust runtime",
      "content": "Tokio is an event-driven, non-blocking I/O platform...",
      "engine": "duckduckgo",
      "engines": ["duckduckgo", "brave"],
      "score": 1.0,
      "category": "general"
    }
  ],
  "unresponsive_engines": [["bing", "Timeout"]]
}
```

計測値が必要なときは `&full=1` を付けると `timings` / `number_of_results` / `engines` が増えます。障害時の切り分けに使えます。

### 3. サイト指定・新着指定が URL 1 本で済む

```bash
# GitHub だけを検索（!サイト名）
curl 'http://127.0.0.1:29483/search?q=%21github+tokio&format=json'

# 24 時間以内の情報だけ（ニュース系エンジンが日付フィルタを解釈）
curl 'http://127.0.0.1:29483/search?q=ai&time_range=day&format=json'

# カテゴリ指定（general / news / images / videos / science / it / files ... 全 47 種）
curl 'http://127.0.0.1:29483/search?q=llm&categories=science&format=json'
```

「まず GitHub だけ見る」「最新ニュースだけ」といったエージェントの判断を、追加の API なしで表現できます。

### 4. 落ちているサイトを自動で切り離す

応答しないサイトは自動でサスペンドし、以降の検索から外します。1 つのサイトの障害でツール全体のレイテンシが崩れません。障害中のエンジンは `unresponsive_engines` に入るので、エージェント側で「この情報源は今回使えなかった」と説明できます。

### 5. 自前のデータも同じ API で引ける

PostgreSQL / MySQL / MariaDB / MongoDB / SQLite / Valkey / Elasticsearch / Solr / Meilisearch を検索対象にできます。社内 Wiki やドキュメントを DB に入れておけば、**Web 検索と社内検索を同じ JSON で**エージェントに渡せます。

```toml
[[engines]]
name = "社内ドキュメント"
engine = "postgresql"
shortcut = "docs"
categories = ["general", "it"]
host = "127.0.0.1"
database = "wiki"
query_str = "SELECT title, body FROM docs WHERE body LIKE '%' || $1 || '%'"
```

### 6. プライバシーとコスト

クエリは各サイトへ直接送られ、広告・追跡・プロファイルはありません。画像や favicon はプロキシ経由なので、利用者の IP も相手サイトに渡りません。自前ホストなので、検索ログが外部ベンダーに渡ることもありません。

## エージェントへの組み込み方

### 関数呼び出しのツール定義

```json
{
  "name": "web_search",
  "description": "複数の検索エンジンを横断して Web を検索する。最新情報が必要なときは time_range を day にする。特定サイトだけを見たいときは query の先頭に !github のように付ける。",
  "parameters": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "検索語。例: 'rust tokio', '!github tokio'" },
      "time_range": { "type": "string", "enum": ["day", "week", "month", "year"] },
      "categories": { "type": "string", "description": "general / news / science / it / files など" },
      "max_results": { "type": "integer", "description": "返す最大件数。トークンを抑えたいときは 5 を指定" },
      "include_domains": { "type": "string", "description": "このドメインだけに絞る（カンマ区切り）" },
      "include_raw_content": { "type": "boolean", "description": "上位 5 件のページ本文も返す" },
      "pageno": { "type": "integer", "minimum": 1 }
    },
    "required": ["query"]
  }
}
```

HTTP API は `GET /search?q=...&format=json` で利用できます。MCP を使う場合は `rustxng mcp` で組み込みの標準入出力サーバーを起動します。`search` は抜粋を返し、`read_page` は同じセッションの検索で返った URL の本文を取得します。出力上限と取得制限は [README](../README.ja.md#mcpサーバー) を参照してください。

### トークンを抑える

エージェントに渡す量はパラメータ 1 つで半分以下にできます。

```bash
# 既定: 全結果（例: 60 KB / 200 件）
curl 'http://127.0.0.1:29483/search?q=rust&format=json'

# max_results: 上位 5 件だけ（例: 4.6 KB / -92%）
curl 'http://127.0.0.1:29483/search?q=rust&format=json&max_results=5'

# include_raw_content: 上位 5 件のページ本文付き（+16 KB / +3.5 秒）
curl 'http://127.0.0.1:29483/search?q=rust&format=json&max_results=5&include_raw_content=1'
```

## 使い始める

```bash
git clone https://github.com/sahenjp/RustXNG.git
cd RustXNG
cargo run --release -- --port 29483
```

Docker では次のコマンドで起動します。

```bash
docker build -t rustxng . && docker run --rm -p 127.0.0.1:29483:29483 rustxng
```

`http://127.0.0.1:29483/` で検索画面、`/search?q=...&format=json` で API です。

## 補足

- **ライセンス**: AGPL-3.0-or-later（SearXNG と同じ）。帰属は NOTICE に記載
- **互換性**: SearXNG の設定と API を基にしています。Google 関連は削除しています。差分は [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md)
- **バイナリサイズ**: 17 MB（DB エンジンを外した場合）/ 27 MB（全部入り）
- **向いていない用途**: 画面デザインは独自なので、SearXNG のテーマをそのまま使いたい場合は不向きです
