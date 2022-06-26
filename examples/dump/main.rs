#[macro_use]
extern crate log;

use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::{self, FromStr};

use anyhow::{bail, Context, Error};
use memmap::Mmap;
use structopt::StructOpt;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Format {
    Text,
    JSON,
    YAML,
    Rust,
}

impl FromStr for Format {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(Format::Text),
            "json" => Ok(Format::JSON),
            "yaml" => Ok(Format::YAML),
            "rust" => Ok(Format::Rust),
            _ => bail!("unknown format: {}", s),
        }
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "btfdump")]
struct Opt {
    /// Output format (c, rust, text, etc.)
    #[structopt(short, long, default_value = "text")]
    format: Format,

    /// Output file
    #[structopt(short, long, parse(from_os_str))]
    output: Option<PathBuf>,

    /// Files to process
    #[structopt(name = "FILE", parse(from_os_str))]
    file: PathBuf,
}

const ANON: &str = "(anon)";

struct TextFmt<'a>(btf::Type<'a>);

impl<'a> fmt::Display for TextFmt<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.0 {
            btf::Type::Void => write!(f, "VOID\n"),
            btf::Type::Integer {
                name,
                size,
                bits_offset,
                nr_bits,
                encoding,
            } => {
                write!(
                    f,
                    "INT '{}' size={} bits_offset={} nr_bits={} encoding={:?}\n",
                    name.unwrap_or(ANON),
                    size,
                    bits_offset,
                    nr_bits,
                    encoding
                )
            }
            btf::Type::Pointer { name, type_id } => {
                write!(f, "PTR '{}' type_id={}\n", name.unwrap_or(ANON), type_id)
            }
            btf::Type::Array {
                name,
                type_id,
                index_type_id,
                nr_elems,
            } => {
                write!(
                    f,
                    "ARRAY '{}' type_id={} index_type_id={} nr_elems={}\n",
                    name.unwrap_or(ANON),
                    type_id,
                    index_type_id,
                    nr_elems
                )
            }
            btf::Type::Struct {
                name,
                size,
                members,
            } => {
                write!(
                    f,
                    "STRUCT '{}' size={} vlen={}\n",
                    name.unwrap_or(ANON),
                    size,
                    members.len()
                )?;

                for m in members {
                    write!(
                        f,
                        "\t'{}' type_id={} bits_offset={}",
                        m.name.unwrap_or(ANON),
                        m.type_id,
                        m.bits_offset
                    )?;

                    if m.bitfield_size != 0 {
                        write!(f, " bitfield_size={}", m.bitfield_size)?;
                    }

                    writeln!(f)?;
                }

                Ok(())
            }
            btf::Type::Union {
                name,
                size,
                members,
            } => {
                write!(
                    f,
                    "UNION '{}' size={} vlen={}\n",
                    name.unwrap_or(ANON),
                    size,
                    members.len()
                )?;

                for m in members {
                    write!(
                        f,
                        "\t'{}' type_id={} bits_offset={}",
                        m.name.unwrap_or(ANON),
                        m.type_id,
                        m.bits_offset
                    )?;

                    if m.bitfield_size != 0 {
                        write!(f, " bitfield_size={}", m.bitfield_size)?;
                    }

                    writeln!(f)?;
                }

                Ok(())
            }
            btf::Type::Enum { name, size, values } => {
                write!(
                    f,
                    "ENUM '{}' size={} vlen={}\n",
                    name.unwrap_or(ANON),
                    size,
                    values.len()
                )?;

                for v in values {
                    write!(f, "\t'{}' val={}\n", v.name.unwrap_or(ANON), v.value)?;
                }

                Ok(())
            }
            btf::Type::Forward { name, kind } => {
                write!(
                    f,
                    "FWD '{}' fwd_kind={}\n",
                    name.unwrap_or(ANON),
                    kind.to_string().to_lowercase()
                )
            }
            btf::Type::Typedef { name, type_id } => {
                write!(
                    f,
                    "TYPEDEF '{}' type_id={}\n",
                    name.unwrap_or(ANON),
                    type_id
                )
            }
            btf::Type::Volatile { name, type_id } => {
                write!(
                    f,
                    "VOLATILE '{}' type_id={}\n",
                    name.unwrap_or(ANON),
                    type_id
                )
            }
            btf::Type::Const { name, type_id } => {
                write!(f, "CONST '{}' type_id={}\n", name.unwrap_or(ANON), type_id)
            }
            btf::Type::Restrict { name, type_id } => {
                write!(
                    f,
                    "RESTRICT '{}' type_id={}\n",
                    name.unwrap_or(ANON),
                    type_id
                )
            }
            btf::Type::Func {
                name,
                type_id,
                linkage,
            } => {
                write!(
                    f,
                    "FUNC '{}' type_id={} linkage={}\n",
                    name.unwrap_or(ANON),
                    type_id,
                    linkage
                )
            }
            btf::Type::FuncProto {
                name,
                ret_type_id,
                params,
            } => {
                write!(
                    f,
                    "FUNC_PROTO '{}' ret_type_id={} vlen={}\n",
                    name.unwrap_or(ANON),
                    ret_type_id,
                    params.len()
                )?;

                for p in params {
                    write!(f, "\t'{}' type_id={}\n", p.name.unwrap_or(ANON), p.type_id,)?;
                }

                Ok(())
            }
            btf::Type::Variable {
                name,
                type_id,
                linkage,
            } => {
                write!(
                    f,
                    "VAR '{}' type_id={} linkage={}\n",
                    name.unwrap_or(ANON),
                    type_id,
                    linkage
                )
            }
            btf::Type::DataSection {
                name,
                size,
                sections,
            } => {
                write!(
                    f,
                    "DATASECTION '{}' size={} vlen={}\n",
                    name.unwrap_or(ANON),
                    size,
                    sections.len()
                )?;

                for s in sections {
                    write!(
                        f,
                        "\ttype_id={} offset={} size={}\n",
                        s.type_id, s.offset, s.size
                    )?;
                }

                Ok(())
            }
            btf::Type::Float { name, size } => {
                write!(f, "FLOAT '{}' size={}\n", name.unwrap_or(ANON), size)
            }
            btf::Type::DeclTag {
                name,
                type_id,
                component_idx,
            } => {
                write!(
                    f,
                    "DECL_TAG '{}' type_id={} component_idx={}\n",
                    name.unwrap_or(ANON),
                    type_id,
                    component_idx
                )
            }
            btf::Type::TypeTag { name, type_id } => {
                write!(
                    f,
                    "TYPE_TAG '{}' type_id={}\n",
                    name.unwrap_or(ANON),
                    type_id
                )
            }
        }
    }
}

fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    let opt = Opt::from_args();
    debug!("opts: {:?}", &opt);

    let mut w = if let Some(path) = opt.output {
        either::Left(File::create(path)?)
    } else {
        either::Right(io::stdout().lock())
    };

    let f = File::open(&opt.file)?;
    let mm = unsafe { Mmap::map(&f)? };

    for (idx, res) in btf::parse(&mm).context("parse BTF file")?.enumerate() {
        let ty = res?;

        match opt.format {
            Format::Text => {
                write!(&mut w, "[{}] {}", idx + 1, TextFmt(ty))?;
            }
            Format::JSON => {
                serde_json::to_writer(&mut w, &ty)?;
                write!(&mut w, "\n")?;
            }
            Format::YAML => {
                serde_yaml::to_writer(&mut w, &ty)?;
            }
            Format::Rust => {}
        }
    }

    Ok(())
}
