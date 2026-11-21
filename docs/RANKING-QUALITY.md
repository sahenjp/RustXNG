# 検索ランキングの品質改善（2026-11-21）

対象: `src/results/`（スコア計算と重複排除）、`src/settings.rs`、`settings.toml`。
検索は `ResultContainer::extend_from_page` で結果を集約し、`close`
（new: `close_with`）でスコアを確定して並べ替える。従来のスコアは
「エンジン重み ÷ 順位」の総和に関連度と権威度を足したもので、次の弱点があった。

- 同じ URL を複数エンジンが返しても、`positions.len()` による重み付け以上の
  「一致」の評価がない。
- タイトル・スニペットが問いにどれだけ合っているかを、URL を含む関連度とは
  別の係数で調整できない。
- 重複排除はスキーム・フラグメント・トラッキングパラメータの正規化だけで、
  AMP・モバイル版・印刷版や、同一ホストでパスだけ違う同一記事が残る。

## 変更内容

### 1. 複数ソース一致ボーナス（`search.consensus_weight`）

`close_with` の位置融合スコアに、次の項を加算する（既存の項は変更しない）。

```text
consensus(n) = (n - 1) / n        （n = その結果を返したエンジン数）
score += consensus_weight * consensus(n)
```

- n = 1 は 0、n = 2 で 0.5、n = 3 で 0.667。上限は 1.0 未満なので、
  エンジン数だけで位置融合を打ち消すことはない。
- 単一エンジン指定（`!engine` など）では 0 にする。既存の relevance/authority と
  同じ扱いで、エンジン自身の順序を尊重する。
- 既定 `consensus_weight = 0.5`。0 で無効。

### 2. タイトル/スニペット一致（`search.text_match_weight`）

URL を含む既存の `relevance_weight` とは別に、タイトル（7 割）と
スニペット先頭 600 文字（3 割）がクエリ語を含む割合を 0〜1 で計算し、
`text_match_weight` を掛けて加算する。

- クエリ語は既存の `query_terms`（3 文字以上の語 + CJK バイグラム）を使う。
- 単一エンジン指定では 0。
- 既定 `text_match_weight = 0.1`。0 で無効。

### 3. 重複排除の強化（`search.dedup_fuzzy`）

`dedup_key` にコピー判定を足し、さらに本文類似による統合を 1 パス追加した。

- URL のコピーマーカー: 末尾の `/amp`・`/print` セグメント（ページが前に
  ある場合のみ）、`.amp.html` / `.print.html` 接尾辞、`amp=1` / `print=1` /
  `output=amp` などのクエリ。`/amp/settings` のような実ページは変更しない。
- 近接重複の統合: 次のすべてを満たす場合だけ 1 件にする。
  - ホストが同じ（`www.` / `m.` / `mobile.` / `amp.` の接頭辞を除いて比較）
  - 正規化タイトルが完全一致
  - 両方のスニペットが正規化後 80 文字以上
  - 文字 4-gram の Dice 係数が 0.9 以上
- 統合時はスコアの高い結果を代表にし、`merged_sources` に元の
  「タイトル | URL」を残す（既存の蒸留と同じ流儀）。URL の代表は既存の
  `merge_result` の規則に従う。
- 既定 `dedup_fuzzy = true`。false で従来の URL 正規化だけに戻る
  （オフライン回帰の基準側はこの設定）。

## 設定の検証

- `Settings::validate` で `consensus_weight` と `text_match_weight` が
  有限かつ 0〜100 であることを検査する。
- `Settings::warnings` は 20 を超える値に対して「位置融合を打ち消し得る」と
  警告する。既定値では警告しない（既存の警告テストも維持）。
- `close` の既存シグネチャは残し、新しい項を 0 として従来と同じスコアを返す。
  新しい項を使う呼び出しは `close_with` と `RankingOptions` を使う。
- `dedup_key` はコピーマーカーのない URL に対して従来と同じ文字列を返す。

## 既定値の根拠（オフライン回帰）

`tests/fixtures/quality/engine-snapshots.json` は、リポジトリに記録済みの
検索実行（`bench/results/issue-resolution-2026-11-05/coverage/standard-1.json.gz`）
から、エンジンごとの応答ウィンドウを再構成した固定入力である。作り方は
[tests/fixtures/quality/README.md](../tests/fixtures/quality/README.md) と
`tools/build_quality_fixture.py` に書いた。実プロバイダーへの問い合わせは
行っていない。

`cargo test --lib results::quality -- --nocapture` の実測（この変更の
スナップショット上。91 問 = 英語 60・日本語 31）:

| 構成 | Hit@1 | Hit@5 | MRR@10 |
| --- | ---: | ---: | ---: |
| 変更前の構成（URL のみの重複排除、新項 0） | 48/91 | 58/91 | 0.5750 |
| 既定（consensus 0.5 + text 0.1） | 49/91 | 58/91 | 0.5804 |

言語別では、英語が Hit@1 33/60 → 34/60、Hit@5 42/60 → 42/60、
MRR 0.6141 → 0.6224。日本語は 15/31・16/31・0.4992 で不変。

既定値の選び方（スナップショット上で測った範囲）:

- `text_match_weight` は 0.1 まで Hit@1/Hit@5/MRR を落とさない。0.2 にすると
  英語の 1 問（TCP/IP の記事）で正解ページが 5 位から 6 位に落ち、Hit@5 が
  58 → 57 になる。0.05 と 0.1 は同じ結果だったので、効果の出る側の 0.1 を採った。
- `text_match_weight` 単独（consensus 0）では 0.1 でも Hit@5 が 1 問落ちる。
  `consensus_weight = 0.5` と組み合わせたときだけ全指標が非減少になった。
  この組み合わせはスナップショット上の測定で選んでおり、実検索での再現は
  確認していない。
- `consensus_weight` は 0.2〜0.6 の範囲でスナップショットの指標を変えなかった。
  0.5 は単独でも順位を大きく崩さず、対象テスト（2 エンジンが 5 位で一致する
  結果が 1 エンジン 1 位を抜く）で意図した順位変化を示せる値として選んだ。
- `dedup_fuzzy` はこのスナップショットの指標を変えなかった（記録済みの
  上位 10 件に AMP・モバイル版・印刷版が含まれていないため）。重複排除の
  規則自体は `src/results/mod.rs` の単体テストで検証している。

**この表は記録の再構成に対する回帰値であり、実検索の品質ではない。**
スナップショットは変更前のランキングで並んだ上位 10 件から作っているため、
新しい項の効果は「その並びをどこまで保てるか」としてしか測れていない。

## 測っていないこと

- 変更前の実プロバイダー比較は行っていない（変更後のライブ測定は
  [BENCHMARK-MULTILINGUAL.md](BENCHMARK-MULTILINGUAL.md) に記録）。60 問の
  Hit@1 は en 8 / ja 2 / ko 1 で、英語以外は検索元の被覆が先に足りない。
- 韓国語の記録済み実行が存在しないため、オフライン回帰に韓国語の設問は入っていない。
  多言語 60 問のうち韓国語 20 問は未測定。
- 重複排除の強化が実結果集合で誤統合を起こさないかは、記録済みスナップショットでは
  検証できていない。規則を保守的にした理由は上記のとおり。
- レイテンシ・メモリへの影響は測っていない。追加は文字列比較と 1 パスの
  グルーピングだけで、I/O は増えない。

## 再現

```bash
python3 tools/build_quality_fixture.py          # フィクスチャの再生成（決定的）
cargo test --lib results::quality -- --nocapture  # 変更前後の数値と回帰検査
cargo test --lib results::tests                    # スコア項と重複排除の単体テスト
```

ライブで測る場合は従来どおり [BENCHMARK-MULTILINGUAL.md](BENCHMARK-MULTILINGUAL.md)
の手順を使い、同じ再起動方針・同じ `--params` で変更前後を比較する。
