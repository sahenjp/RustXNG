# SealQA 検索ベンチマーク — 2026-11-02

この記録は当時の既定エンジン構成による測定であり、[プロバイダー設定の変更](PROVIDERS.md)後の性能を示さない。
Yahoo! JAPANの1位結果の件数は寄与の記録であり、除外時の性能低下量を測ったものではない。

RustXNG単体の検索を評価した。公式SealQAの回答正解率ではなく、データセットの参照ページが
検索上位に入る割合を測る。回答生成・LLM採点・本文取得は行っていない。

## 結果

| split | 問数 | 参照ページ@1 | @5 | @10 | MRR@10 | 中央値 | p95 | リクエストエラー |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| seal_0 | 111 | 47.75% (53) | 54.95% (61) | 65.77% (73) | 0.5215 | 1.848s | 2.762s | 0 |
| seal_hard | 254 | 39.37% (100) | 49.21% (125) | 61.42% (156) | 0.4468 | 1.528s | 2.834s | 0 |

全254問で結果が返り、0件応答はなかった。リクエスト成功は全エンジンの成功を意味しない。
未応答・休止等を含むunresponsive_enginesの中央値はSeal-0で38、Seal-Hardで39だった。

## 条件

- 対象: main `fef998b69cc1882a9258a7f2cba59bf77793d729`、releaseビルド、Rust 1.99.1 / Linux。
- データ: [公式 vtllms/sealqa](https://huggingface.co/datasets/vtllms/sealqa)、
  revision `0c298f519c871579c75920189b3858da29fe2c81`、Apache-2.0。
- Seal-0 111問はSeal-Hard 254問に含まれる。各質問を一度だけ検索し、同一観測から両splitを集計した。
  独立した2回の実験ではない。Seal-0を先に実行し、残りの143問をその後に実行した。
- 既定settings.toml。登録エンジン171、検索カテゴリgeneral、strategy=parallel、検索期限3秒。
- 新規の専用サーバーを1つ起動して連続測定。自動休止・キャッシュを含む通常の挙動を維持した。
  途中再起動・結果に合わせた設定変更はしていない。
- 逐次リクエスト、間隔0.25秒、クライアント期限20秒。各質問は原文のまま送信した。
- `/search?format=json&max_results=10&full=1`。ローカルインデックス・embeddings・LLMなし。
- goldの回答と参照URLは検索語に混ぜず、取得後の採点だけに使った。
- scheme、www、末尾slash、fragmentを除いてページ単位で一致判定。
  パスの大文字小文字・queryは保持し、URLの単純な前方一致は使わない。
- hit@kは参照URLのいずれかが上位k件に入った問数/全問数。
  MRR@10は10位以内の最初の一致順位の逆数の平均。エラーも分母に含める。

## 読み取り方と制約

Seal-Hardでは参照ページ156/254問に上位10件で到達し、98問では到達しなかった。
別の有効なページで答えられる場合も、この厳格な参照URL指標ではmissになる。
逆に参照ページが返っても、ページ本文に必要な根拠があることや正答できることは保証しない。

取得日時・地域・IP・上流のブロック状態に依存する1回の測定であり、信頼区間や複数回の分散は未測定。
動的な質問と参照ページはデータセットの固定版に従っており、現在の回答の正しさは評価していない。
公式リーダーボードや既存の100問ベンチと数値を直接比較しない。

結果URLにSealQA論文ページが1件含まれた。ベンチマーク関連URLの検出結果を各問に記録し、
この測定では削除や恣意的な順位変更はしていない。回答文の一致を指標にしていないため、
ベンチマークの答えが記載されたページを返すだけで正解とは数えない。
LongSealは提供された長いコンテキストを扱う別の評価なので、今回の検索単体測定には含めていない。

## 寄与したエンジン

上位1件の由来（重複統合時はAPIが返す代表engine）:

| エンジン | 問数 |
| --- | ---: |
| yahoo japan | 189 |
| swisscows | 31 |
| seznam | 20 |
| keenable | 13 |
| naver | 1 |

これは各エンジン単独の精度ではなく、統合・ランキング後の先頭結果の出所である。
Yahoo! JAPANへの寄与の偏りがあるため、既存Issue #16のBing市場別対応は、
既定設定との同一条件比較として測定する価値がある。

## 再実行

```bash
# Python側の追加依存
python3 -m pip install pyarrow

cargo build --release --locked --bin rustxng
./target/release/rustxng --bind 127.0.0.1 --port 18991
# 別のターミナルから
python3 tools/eval_sealqa.py --base http://127.0.0.1:18991 --output bench/sealqa-new-run
python3 -m unittest discover -s tools -p test_eval_sealqa.py
```

データセットはユーザーcacheへ保存する。生parquetやgold回答はリポジトリへ複製していない。

## 成果物と検証

- [全問の検索結果と参照URL](../bench/sealqa-2026-11-02/observations.jsonl)
- [集計JSON](../bench/sealqa-2026-11-02/summary.json)
- [条件とデータSHA-256](../bench/sealqa-2026-11-02/metadata.json)
- [バイナリ・runnerのSHA-256と実行環境](../bench/sealqa-2026-11-02/provenance.json)
- [サーバー設定情報](../bench/sealqa-2026-11-02/server-config.json)
- [実行終了時メトリクス](../bench/sealqa-2026-11-02/metrics.txt)
- [採点テスト](../bench/sealqa-2026-11-02/scoring-tests.log)
- [保存URLからの再計算照合](../bench/sealqa-2026-11-02/verification.log)
- [実行スクリプト](../tools/eval_sealqa.py)

採点の7テストが成功し、最終runnerで保存URLから両splitのスコアを再計算して完全一致を確認した。
