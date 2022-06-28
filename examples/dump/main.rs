use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::{self, FromStr};

use anyhow::{bail, Error};
use log::debug;
use memmap::Mmap;
use serde::Serialize;
use structopt::StructOpt;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Format {
    Text,
    JSON,
    PrettyJSON,
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
    /// Generate text output.
    #[structopt(short, long)]
    text: bool,

    /// Generate JSON output.
    #[structopt(short, long)]
    json: bool,

    /// Generate human-readable JSON output.
    #[structopt(short, long)]
    pretty: bool,

    /// Generate YAML output.
    #[structopt(short, long)]
    yaml: bool,

    /// Generate Rust output.
    #[structopt(short, long)]
    rust: bool,

    /// Output format (text, json, yaml or rust)
    #[structopt(short, long, default_value = "text")]
    format: Format,

    /// Output file
    #[structopt(short, long, parse(from_os_str))]
    output: Option<PathBuf>,

    /// Pass a base BTF object.
    #[structopt(short, long, parse(from_os_str))]
    base_btf: Option<PathBuf>,

    /// Files to process
    #[structopt(name = "FILE", parse(from_os_str))]
    file: PathBuf,
}

impl Opt {
    pub fn format(&self) -> Format {
        if self.text {
            Format::Text
        } else if self.pretty {
            Format::PrettyJSON
        } else if self.json {
            Format::JSON
        } else if self.yaml {
            Format::YAML
        } else if self.rust {
            Format::Rust
        } else {
            self.format
        }
    }
}

const ANON: &str = "(anon)";

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Types<'a> {
    pub types: Vec<Type<'a>>,
}

impl<'a> Types<'a> {
    pub fn new(types: btf::Types<'a>) -> Result<Types<'a>, Error> {
        Ok(Types {
            types: types
                .enumerate()
                .map(|(idx, res)| res.map(|ty| Type { id: idx + 1, ty }))
                .collect::<Result<Vec<_>, btf::Error>>()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Type<'a> {
    pub id: usize,
    #[serde(flatten)]
    pub ty: btf::Type<'a>,
}

struct TextFmt<'a>(&'a Type<'a>, &'a [Type<'a>]);

impl<'a> fmt::Display for TextFmt<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}] ", self.0.id)?;

        match &self.0.ty {
            btf::Type::Void => write!(f, "VOID\n"),
            btf::Type::Int {
                name,
                size,
                bits_offset,
                nr_bits,
                encoding,
            } => {
                write!(
                    f,
                    "INT '{}' size={} bits_offset={} nr_bits={} encoding={:?}\n",
                    name, size, bits_offset, nr_bits, encoding
                )
            }
            btf::Type::Ptr { type_id } => {
                write!(f, "PTR '{}' type_id={}\n", ANON, type_id)
            }
            btf::Type::Array {
                type_id,
                index_type_id,
                nr_elems,
            } => {
                write!(
                    f,
                    "ARRAY '{}' type_id={} index_type_id={} nr_elems={}\n",
                    ANON, type_id, index_type_id, nr_elems
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
                    write!(f, "\t'{}' val={}\n", v.name.unwrap_or(ANON), v.val)?;
                }

                Ok(())
            }
            btf::Type::Fwd { name, fwd_kind } => {
                write!(
                    f,
                    "FWD '{}' fwd_kind={}\n",
                    name,
                    fwd_kind.to_string().to_lowercase()
                )
            }
            btf::Type::Typedef { name, type_id } => {
                write!(f, "TYPEDEF '{}' type_id={}\n", name, type_id)
            }
            btf::Type::Volatile { type_id } => {
                write!(f, "VOLATILE '{}' type_id={}\n", ANON, type_id)
            }
            btf::Type::Const { type_id } => {
                write!(f, "CONST '{}' type_id={}\n", ANON, type_id)
            }
            btf::Type::Restrict { type_id } => {
                write!(f, "RESTRICT '{}' type_id={}\n", ANON, type_id)
            }
            btf::Type::Func {
                name,
                type_id,
                linkage,
            } => {
                write!(
                    f,
                    "FUNC '{}' type_id={} linkage={}\n",
                    name, type_id, linkage
                )
            }
            btf::Type::FuncProto {
                ret_type_id,
                params,
            } => {
                write!(
                    f,
                    "FUNC_PROTO '{}' ret_type_id={} vlen={}\n",
                    ANON,
                    ret_type_id,
                    params.len()
                )?;

                for p in params {
                    write!(f, "\t'{}' type_id={}\n", p.name.unwrap_or(ANON), p.type_id)?;
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
                    "VAR '{}' type_id={}, linkage={}\n",
                    name, type_id, linkage
                )
            }
            btf::Type::DataSec {
                name,
                size,
                sections,
            } => {
                write!(
                    f,
                    "DATASEC '{}' size={} vlen={}\n",
                    name,
                    size,
                    sections.len()
                )?;

                for s in sections {
                    write!(
                        f,
                        "\ttype_id={} offset={} size={} (VAR '{}')\n",
                        s.type_id,
                        s.offset,
                        s.size,
                        if let btf::Type::Variable { name, .. } =
                            self.1[(s.type_id - 1) as usize].ty
                        {
                            name
                        } else {
                            "UNKNOWN"
                        }
                    )?;
                }

                Ok(())
            }
            btf::Type::Float { name, size } => {
                write!(f, "FLOAT '{}' size={}\n", name, size)
            }
            btf::Type::DeclTag {
                name,
                type_id,
                component_idx,
            } => {
                write!(
                    f,
                    "DECL_TAG '{}' type_id={} component_idx={}\n",
                    name, type_id, component_idx
                )
            }
            btf::Type::TypeTag { name, type_id } => {
                write!(f, "TYPE_TAG '{}' type_id={}\n", name, type_id)
            }
        }
    }
}

fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    let opt = Opt::from_args();
    debug!("opts: {:?}", &opt);

    let format = opt.format();

    let mut w = if let Some(path) = opt.output {
        either::Left(File::create(path)?)
    } else {
        either::Right(io::stdout().lock())
    };

    let f = File::open(&opt.file)?;
    let mm = unsafe { Mmap::map(&f)? };
    let types = btf::parse(&mm)?;

    let base_btf = opt
        .base_btf
        .map(|file| -> Result<_, Error> {
            let f = File::open(file)?;
            let mm = unsafe { Mmap::map(&f)? };
            Ok(mm)
        })
        .transpose()?;
    let base_types = base_btf
        .as_ref()
        .map(|mm| btf::parse(&mm)?.collect())
        .transpose()?;

    match format {
        Format::JSON => {
            serde_json::to_writer(&mut w, &Types::new(types)?)?;
        }
        Format::PrettyJSON => {
            serde_json::to_writer_pretty(&mut w, &Types::new(types)?)?;
        }
        Format::YAML => {
            serde_yaml::to_writer(&mut w, &Types::new(types)?)?;
        }
        Format::Text => {
            let types = Types::new(types)?;

            for res in &types.types {
                write!(&mut w, "{}", TextFmt(res, &types.types))?;
            }
        }
        Format::Rust => {
            let types = types.collect::<Result<Vec<_>, btf::Error>>()?;

            let src = btf::rust::dump(base_types.as_ref().map(Vec::as_slice), types.as_slice());

            w.write_all(src.as_bytes())?;
        }
    }

    Ok(())
}
