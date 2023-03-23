import { assert, expect } from "chai";
import { SystemProgram, Keypair as SolanaKeypair } from "@solana/web3.js";
import * as anchor from "@coral-xyz/anchor";
import { it } from "mocha";
import { buildPoseidonOpt } from "circomlibjs";

import { Account } from "../src/account";
import { Utxo } from "../src/utxo";
import {
  FEE_ASSET,
  hashAndTruncateToCircuit,
  Provider as LightProvider,
  MINT,
  Relayer,
  UtxoError,
  UtxoErrorCode,
} from "../src";
process.env.ANCHOR_PROVIDER_URL = "http://127.0.0.1:8899";
process.env.ANCHOR_WALLET = process.env.HOME + "/.config/solana/id.json";

describe("Utxo Functional", () => {
  let seed32 = new Uint8Array(32).fill(1).toString();
  let depositAmount = 20_000;
  let depositFeeAmount = 10_000;

  let mockPubkey = SolanaKeypair.generate().publicKey;
  let mockPubkey1 = SolanaKeypair.generate().publicKey;
  let mockPubkey2 = SolanaKeypair.generate().publicKey;
  let mockPubkey3 = SolanaKeypair.generate().publicKey;
  let poseidon, lightProvider, deposit_utxo1, outputUtxo, relayer, keypair;
  before(async () => {
    poseidon = await buildPoseidonOpt();
    // TODO: make fee mandatory
    relayer = new Relayer(
      mockPubkey3,
      mockPubkey,
      mockPubkey,
      new anchor.BN(5000),
    );
    keypair = new Account({ poseidon: poseidon, seed: seed32 });
    lightProvider = await LightProvider.loadMock(mockPubkey3);
    deposit_utxo1 = new Utxo({
      poseidon: poseidon,
      assets: [FEE_ASSET, MINT],
      amounts: [new anchor.BN(depositFeeAmount), new anchor.BN(depositAmount)],
      account: keypair,
      index: 1,
    });
  });

  it("rnd utxo functional", async () => {
    // try basic tests for rnd empty utxo
    const utxo4 = new Utxo({ poseidon });
    // toBytes
    const bytes4 = utxo4.toBytes();
    // fromBytes
    const utxo40 = Utxo.fromBytes({
      poseidon,
      account: utxo4.account,
      bytes: bytes4,
      index: 0,
    });
    Utxo.equal(utxo4, utxo40);
    // encrypt
    const encBytes4 = utxo4.encrypt();
    const utxo41 = Utxo.decrypt({
      poseidon,
      encBytes: encBytes4,
      account: utxo4.account,
      index: 0,
    });
    if (utxo41) {
      Utxo.equal(utxo4, utxo41);
    } else {
      throw "decrypt failed";
    }
  });

  it("encryption", async () => {
    const amountFee = "1";
    const amountToken = "2";
    const assetPubkey = MINT;
    const seed32 = new Uint8Array(32).fill(1).toString();
    let inputs = {
      keypair: new Account({ poseidon, seed: seed32 }),
      amountFee,
      amountToken,
      assetPubkey,
      assets: [SystemProgram.programId, assetPubkey],
      amounts: [new anchor.BN(amountFee), new anchor.BN(amountToken)],
      blinding: new anchor.BN(new Uint8Array(31).fill(2)),
      index: 1,
    };

    let utxo0 = new Utxo({
      poseidon,
      assets: inputs.assets,
      amounts: inputs.amounts,
      account: inputs.keypair,
      blinding: inputs.blinding,
      index: inputs.index,
    });
    // functional
    assert.equal(utxo0.amounts[0].toString(), amountFee);
    assert.equal(utxo0.amounts[1].toString(), amountToken);
    assert.equal(
      utxo0.assets[0].toBase58(),
      SystemProgram.programId.toBase58(),
    );
    assert.equal(utxo0.assets[1].toBase58(), assetPubkey.toBase58());
    assert.equal(
      utxo0.assetsCircuit[0].toString(),
      hashAndTruncateToCircuit(SystemProgram.programId.toBytes()).toString(),
    );
    assert.equal(
      utxo0.assetsCircuit[1].toString(),
      hashAndTruncateToCircuit(assetPubkey.toBytes()).toString(),
    );
    assert.equal(utxo0.instructionType.toString(), "0");
    assert.equal(utxo0.poolType.toString(), "0");
    assert.equal(
      utxo0.verifierAddress.toString(),
      "11111111111111111111111111111111",
    );
    assert.equal(utxo0.verifierAddressCircuit.toString(), "0");
    assert.equal(
      utxo0.getCommitment()?.toString(),
      "652669139698397343583748072204170820200438709928429876748650598683161543212",
    );

    assert.equal(
      utxo0.getNullifier()?.toString(),
      "21628460042151823012568567445627492316110015914090115121984057579010072862243",
    );

    // toBytes
    const bytes = utxo0.toBytes();
    // fromBytes
    const utxo1 = Utxo.fromBytes({
      poseidon,
      account: inputs.keypair,
      bytes,
      index: inputs.index,
    });
    Utxo.equal(utxo0, utxo1);
    // encrypt
    const encBytes = utxo1.encrypt();

    // decrypt
    const utxo3 = Utxo.decrypt({
      poseidon,
      encBytes,
      account: inputs.keypair,
      index: inputs.index,
    });
    if (utxo3) {
      Utxo.equal(utxo0, utxo3);
    } else {
      throw "decrypt failed";
    }
  });
});

describe("Utxo Errors", () => {
  let seed32 = new Uint8Array(32).fill(1).toString();

  let poseidon, inputs, keypair;

  const amountFee = "1";
  const amountToken = "2";
  const assetPubkey = MINT;

  before(async () => {
    poseidon = await buildPoseidonOpt();
    keypair = new Account({ poseidon: poseidon, seed: seed32 });
    inputs = {
      keypair: new Account({ poseidon, seed: seed32 }),
      amountFee,
      amountToken,
      assetPubkey,
      assets: [SystemProgram.programId, assetPubkey],
      amounts: [new anchor.BN(amountFee), new anchor.BN(amountToken)],
      blinding: new anchor.BN(new Uint8Array(31).fill(2)),
    };
  });

  it("get nullifier without index", async () => {
    let pubkeyUtxo = new Utxo({
      poseidon,
      amounts: [new anchor.BN(1)],
      account: Account.fromPubkey(
        keypair.pubKey,
        keypair.encryptionKeypair.publicKey,
        poseidon,
      ),
    });

    expect(() => {
      pubkeyUtxo.getNullifier();
    })
      .throw(UtxoError)
      .include({
        code: UtxoErrorCode.INDEX_NOT_PROVIDED,
        functionName: "getNullifier",
      });
  });

  it("get nullifier without private key", async () => {
    let pubkeyUtxo = new Utxo({
      poseidon,
      amounts: [new anchor.BN(1)],
      account: Account.fromPubkey(
        keypair.pubKey,
        keypair.encryptionKeypair.publicKey,
        poseidon,
      ),
      index: 1,
    });

    expect(() => {
      pubkeyUtxo.getNullifier();
    })
      .throw(UtxoError)
      .include({
        code: UtxoErrorCode.ACCOUNT_HAS_NO_PRIVKEY,
        functionName: "getNullifier",
      });
  });

  it("INVALID_ASSET_OR_AMOUNTS_LENGTH", () => {
    expect(() => {
      new Utxo({
        poseidon,
        assets: [inputs.assets[1]],
        amounts: inputs.amounts,
        account: inputs.keypair,
        blinding: inputs.blinding,
      });
    })
      .to.throw(UtxoError)
      .to.include({
        code: UtxoErrorCode.INVALID_ASSET_OR_AMOUNTS_LENGTH,
        codeMessage: "Length missmatch assets: 1 != amounts: 2",
      });
  });

  it("EXCEEDED_MAX_ASSETS", () => {
    expect(() => {
      new Utxo({
        poseidon,
        assets: [MINT, MINT, MINT],
        amounts: [new anchor.BN(1), new anchor.BN(1), new anchor.BN(1)],
        account: inputs.keypair,
        blinding: inputs.blinding,
      });
    })
      .to.throw(UtxoError)
      .to.include({
        code: UtxoErrorCode.EXCEEDED_MAX_ASSETS,
        codeMessage: "assets.length 3 > N_ASSETS 2",
      });
  });

  it("NEGATIVE_AMOUNT", () => {
    expect(() => {
      new Utxo({
        poseidon,
        assets: inputs.assets,
        amounts: [inputs.amounts[0], new anchor.BN(-1)],
        account: inputs.keypair,
        blinding: inputs.blinding,
      });
    })
      .to.throw(UtxoError)
      .to.include({
        code: UtxoErrorCode.NEGATIVE_AMOUNT,
        codeMessage: "amount cannot be negative, amounts[1] = -1",
      });
  });

  it("APP_DATA_FROM_BYTES_FUNCTION_UNDEFINED", () => {
    expect(() => {
      new Utxo({
        poseidon,
        assets: inputs.assets,
        amounts: inputs.amounts,
        account: inputs.keypair,
        blinding: inputs.blinding,
        appData: new Array(32).fill(1),
      });
    })
      .to.throw(UtxoError)
      .to.include({
        code: UtxoErrorCode.APP_DATA_FROM_BYTES_FUNCTION_UNDEFINED,
        codeMessage: "No appDataFromBytesFn provided",
      });
  });

  it("INVALID_ASSET_OR_AMOUNTS_LENGTH", () => {
    expect(() => {
      new Utxo({
        poseidon,
        assets: [SystemProgram.programId, SolanaKeypair.generate().publicKey],
        amounts: inputs.amounts,
        account: inputs.keypair,
        blinding: inputs.blinding,
      }).toBytes();
    })
      .to.throw(UtxoError)
      .to.include({
        code: UtxoErrorCode.ASSET_NOT_FOUND,
        functionName: "toBytes",
      });
  });
});