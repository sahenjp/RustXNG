# 多言語ベンチマーク（日本語・韓国語・英語）

エージェントがコードを書く前に引く**技術ドキュメントの検索**を、3 言語で同じ手順で測るための設問セットです。
設問データは [bench/quality-multilingual.json](../bench/quality-multilingual.json)、評価器は
[tools/eval_quality.py](../tools/eval_quality.py) です。

既存の [BENCHMARK.md](BENCHMARK.md) が英語・日本語の 20 問／100 問セットでの記録なのに対し、
この文書は**韓国語を含む 60 問**の条件と実行方法を定めます。
**この文書には新しい実測値を載せていません。** 理由は「[測定値の扱い](#測定値の扱い)」に書きます。

## 構成

| 言語 | 設問 | id 接頭辞 |
| --- | ---: | --- |
| 日本語（`lang: "ja"`） | 20 | `ja-ml-` |
| 韓国語（`lang: "ko"`） | 20 | `ko-ml-` |
| 英語（`lang: "en"`） | 20 | `en-ml-` |

- カテゴリ: `doc` 55 / `official` 5（ベンダー公式サイトの入口）
- 正解（`gold`）: 1〜2 個の URL 前置き
- 設問は「実装前に公式ドキュメントを引く」形に限定しています（Python / Rust / JavaScript / Kubernetes / Docker / Git / PostgreSQL / Django / FastAPI / クラウドの公式ドキュメントなど）
- 1 エントリの形は既存ファイルと同じです

```json
{
  "id": "ja-ml-asyncio",
  "lang": "ja",
  "query": "python asyncio 非同期 ドキュメント",
  "gold": ["https://docs.python.org/ja/3/library/asyncio.html"],
  "category": "doc"
}
```

## 正解 URL の決め方

- 設問の作成に検索プロバイダーは使っていません。gold は公開ドキュメントの URL を直接開いて確認しました
  （2026-11-21 時点で全 55 件が HTTP 200）。
- ローカライズ版が安定して存在する場合はその言語のページ、無い場合は英語の正規ページを gold にしています。
  韓国語の一部は、同じ内容の英語ページも gold に併記しています（どちらに着地しても正解）。
- 照合は [tools/reference_urls.py](../tools/reference_urls.py) の前置き一致です。スキーム・フラグメント・
  バージョン相当のパス断片は無視し、パスの境界で判定します。拡張子なしの gold は `.html` / `.htm` も受けます。
- gold は「ページまたはディレクトリの前置き」であり、取得して本文を照合するものではありません。
  設問を追加するときも、検索結果から逆算せず、公式ドキュメントの URL を直接確認してください。

## 実行方法

すでに起動しているインスタンスを測る場合:

```bash
python3 tools/eval_quality.py --base http://127.0.0.1:8888 \
  --questions bench/quality-multilingual.json --by-lang \
  --delay 1.1 --json /tmp/multilingual.json
```

インスタンスが無ければ先に起動します（release ビルド）。

```bash
cargo build --release
./target/release/rustxng --port 8888
```

`--by-lang` を付けると、全体の集計に加えて言語ごとの hit@1 / hit@5 / MRR@10 / 中央値が出ます。
付けない場合の標準出力と JSON の形は従来と同じなので、既存ファイルで記録した数値とそのまま比較できます。
付けた場合、JSON に `by_language` が増え、`rows` の各要素は元の `lang` を保持します。

```text
questions=60  hit@1=...  hit@5=...  MRR@10=...
median latency=... ms  median response=... B  errors=...
by language:
  ja       questions=20  hit@1=.../20  hit@5=.../20  MRR@10=...  median=... ms
  ko       questions=20  ...
  en       questions=20  ...
```

### 再起動しながら測る（`--restart-every`）

連続実行すると、その計測自体がエンジンのレート制限を蓄積させ、後半の設問が不当に落ちます。
`--restart-binary` は一定問数ごとにプロセスを作り直してこれを避けます。

```bash
python3 tools/eval_quality.py --port 8888 \
  --restart-binary "$(pwd)/target/release/rustxng" \
  --restart-every 10 \
  --questions bench/quality-multilingual.json --by-lang --delay 1.0 \
  --json "bench/results/multilingual-$(date +%F).json"
```

- ポートを `fuser -k` で解放してからバイナリを起動し、`/healthz` が 200 を返すまで待ちます。
  `fuser`（psmisc）が必要で、無い場合は測定を続けず**終了コード 2** で止まります。
- 最初のバッチも起動し直します。`--base` は `--port` の値で上書きされます。
- 60 問なら `--restart-every 20` で 3 回、`10` で 6 回の再起動です。
  再起動が多いほど休止の持ち越しは減りますが、実行時間は伸びます。
- 再起動なしは実運用に近い条件、ありは設問間の公平性を優先した条件です。
  **比較するときは同じ再起動方針で揃えてください。**
- `--delay` は設問間の待機秒です。既定は 1.0 秒で、外部プロバイダーへの負荷を抑えます。

## ばらつき（重要）

数値は**外部検索プロバイダーの稼働状況で大きく動きます**。
[BENCHMARK.md](BENCHMARK.md) の 100 問では、同一設定・同一設問でも hit@1 が 55〜68、
hit@5 が 73〜82 の幅でした。

- 1 回の測定で 2〜3 問の差を結論にしないでください。3 回以上測り、中央値か平均で見ます。
- 言語間の比較は**同じ実行の `by_language`** で行います。別日に別々に測った値を並べると、
  差が言語ではなくその日のエンジン状況を反映します。
- 記録する項目: 日時、バイナリのコミット、再起動方針、`--params`、`--delay`、
  プロバイダーのブロック状況（利用できないエンジンが多い環境では数値は下限になります）。
- 応答が返らない設問も 0 件として集計されます（`errors` を見てください）。
  エラーが多い実行は品質比較に使えません。

## 測定値の扱い

この設問セットを追加した変更では、**ライブのプロバイダーに対する測定を実行していません**。
CI の nightly は外部プロバイダーへ問い合わせない方針で（[BENCHMARK.md](BENCHMARK.md) の
「現在の CI の範囲」）、この環境の数値も時間帯で再現しないためです。
したがって、この文書に 60 問のスコアは載せていません。未測定であることを明示します。

手元で再現した場合は、実行条件とセットで保存してください。

```bash
mkdir -p bench/results
python3 tools/eval_quality.py --port 8888 \
  --restart-binary "$(pwd)/target/release/rustxng" --restart-every 10 \
  --questions bench/quality-multilingual.json --by-lang --delay 1.0 \
  --json "bench/results/multilingual-$(date +%F).json"
```

終了コードは `errors` が 1 件以上あると 1 です。0 件の実行だけを記録に使います。

## オフライン回帰（2026-11-21 追加）

ランキングと重複排除の変更（[RANKING-QUALITY.md](RANKING-QUALITY.md)）では、
外部プロバイダーに問い合わせない固定フィクスチャで変更前後を比較できるように
しました。記録済みの検索実行からエンジンごとの応答ウィンドウを再構成した
`tests/fixtures/quality/engine-snapshots.json` を使います。

```bash
cargo test --lib results::quality -- --nocapture
```

このフィクスチャは英語 60 問・日本語 31 問で、**韓国語の記録済み実行が無いため
60 問多言語セットの全体は含みません**。表に出る Hit@1 / Hit@5 / MRR は
記録の再構成に対する回帰値であり、ライブの検索品質ではありません。
## ライブ測定（2026-11-21、standard プロファイル）

この環境は外部ネットワークに出られるため、実起動のインスタンスに 60 問を実行しました。

| 言語 | 問数 | Hit@1 | Hit@5 |
| --- | ---: | ---: | ---: |
| en | 20 | 8 | 9 |
| ja | 20 | 2 | 2 |
| ko | 20 | 1 | 2 |
| 全体 | 60 | 11 | 13 |

MRR@10 は全体 0.200（en 0.425 / ja 0.100 / ko 0.075）。応答は p50 434ms、
p95 1124ms で、0 件応答が 19 問（ko 13、ja 6、en 0）。

英語以外の低さはランキングではなく検索元の被覆によるものです。ハングル検出の
修正後、韓国語の空応答は 13 → 10 問に減りましたが、Hit@1 / Hit@5 / MRR は
変わりませんでした。

## 検証

```bash
python3 -c 'import json;json.load(open("bench/quality-multilingual.json"))'
python3 -m unittest discover -s tools -p 'test_eval*.py'
python3 tools/test_doc_links.py
```

1 つ目は設問ファイルが JSON として読めること、2 つ目は評価器の回帰、
3 つ目はこの文書を含む相対リンクの解決を確認します。
