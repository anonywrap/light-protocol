use std::{fs::File, io::prelude::*, path::PathBuf};

use clap::{Parser, ValueEnum};
use light_hasher::{zero_bytes::MAX_HEIGHT, Hasher, Poseidon, Sha256};
use light_utils::rustfmt;
use quote::quote;

#[derive(Debug, Clone, ValueEnum)]
enum Hash {
    Sha256,
    Poseidon,
}

#[derive(Debug, Parser)]
pub struct Options {
    #[clap(value_enum, long, default_value_t = Hash::Sha256)]
    hash: Hash,
    #[clap(long)]
    path: Option<PathBuf>,
}

pub fn generate_zero_bytes(opts: Options) -> anyhow::Result<()> {
    match opts.hash {
        Hash::Sha256 => generate_zero_bytes_for_hasher::<Sha256>(opts),
        Hash::Poseidon => generate_zero_bytes_for_hasher::<Poseidon>(opts),
    }
}

fn generate_zero_bytes_for_hasher<H>(opts: Options) -> anyhow::Result<()>
where
    H: Hasher,
{
    let mut zero_bytes = [[0u8; 32]; MAX_HEIGHT + 1];
    let mut zero_bytes_tokens = vec![];

    let mut prev_hash = H::hashv(&[&[1u8; 32], &[1u8; 32]]).unwrap();

    for zero_bytes_element in zero_bytes.iter_mut() {
        let cur_hash = H::hashv(&[&prev_hash, &prev_hash]).unwrap();
        zero_bytes_element.copy_from_slice(&cur_hash);

        let cur_hash_iter = cur_hash.iter();
        zero_bytes_tokens.push(quote! {
            [ #(#cur_hash_iter),* ]
        });

        prev_hash = cur_hash;
    }

    // NOTE(vadorovsky): I couldn't find any way to do a double repetition
    // over a 2D array inside `quote` macro, that's why arrays are converted
    // to tokens in the loop above. But I would be grateful if there is any
    // way to make it prettier.
    //
    // Being able to do something like:
    //
    // ```rust
    // let code = quote! {
    //     const ZERO_BYTES: ZeroBytes = [ #([ #(#zero_bytes),* ]),* ];
    // };
    // ```
    //
    // would be great.
    let code = quote! {
        use super::ZeroBytes;

        pub const ZERO_BYTES: ZeroBytes = [ #(#zero_bytes_tokens),* ];
    };

    println!(
        "Zero bytes (generated with {:?} hash): {:?}",
        opts.hash, zero_bytes
    );

    if let Some(path) = opts.path {
        let mut file = File::create(&path)?;
        file.write_all(b"// This file is generated by xtask. Do not edit it manually.\n\n")?;
        file.write_all(&rustfmt(code.to_string())?)?;
        println!("Zero bytes written to {:?}", path);
    }

    Ok(())
}