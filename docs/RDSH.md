# rdsh から使う

rdsh は `search-web` のバックエンドとして RustXNG を使えます。
このチェックアウトの既定ポートは29483です。旧来の8888探索に依存せず、`RUSTXNG_URL`を明示してください。

## 使い方

```bash
# 1. 新旧バイナリで同じポートを明示
rustxng --bind 127.0.0.1 --port 29483

# 2. rdsh から検索
RUSTXNG_URL=http://127.0.0.1:29483 rdsh search-web --engine rustxng "python dict get"
RUSTXNG_URL=http://127.0.0.1:29483 rdsh search-web --engine rustxng --json "python dict get"
```

## 解決順

`rdsh search-web --engine auto`（既定）は次の順に探します。

1. `SEARXNG_URL`
2. `RUSTXNG_URL`
3. ローカルの `rustxng` コマンド
4. `http://127.0.0.1:8888`

RustXNG を別ホスト・別ポートで動かす場合は、次のように指定します。

```bash
RUSTXNG_URL=http://127.0.0.1:9999 rdsh search-web --engine rustxng "query"
```

## バイナリの導入

`rdsh rustxng install` は GitHub Releases の最新版を `~/.local/bin` へ入れます。RustXNG v0.26.0 の配布物は5プラットフォーム（Linux gnu/musl、macOS aarch64/x86_64、Windows）です。Linux ARM64 GNU 向けの CI とリリース設定も追加しましたが、導入時は対象バージョンの配布物を確認してください。

## 互換性

rdsh は SearXNG の JSON API を読むため、RustXNG は同じ形で応答します。

- `/search?q=...&format=json` の `results[]`（`title` / `url` / `content`）
- `/healthz`（起動確認）
- `unresponsive_engines` は `[engine, 理由]` の配列

2026-10-31 時点で、`rdsh search-web --engine rustxng --json` が RustXNG の応答をそのまま解釈できることを実機で確認しています。

## rdsh と併用するときの推奨設定

- 自前インデックスは任意です。`search.local_index_path`を設定した場合だけ利用します。ディレクトリを置くだけでは有効になりません。
- rdsh は同じクエリを繰り返しがちです。既定のキャッシュ（300 秒）で 2 回目以降は 16 ms で返ります
- `--limit` はrdsh 側の件数指定に合わせて `max_results` が渡ります

## サイドカーの後始末

rdsh は `search-web` のたびに RustXNG をサイドカーとして起動します。環境によってはそのプロセスが残り続けることがあるため、**アイドル終了**を用意しています。

```bash
# 5 分間リクエストが無ければ自動で終了する
RUSTXNG_IDLE_TIMEOUT=300 rustxng
```

`settings.toml` の `server.idle_timeout` でも指定できます（既定 0 = 無効）。
