// ASCII Armor: binary to text encoding library and command-line utility.
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2024 by
//     Dr. Maxim Orlovsky <orlovsky@ubideco.org>
//
// Copyright 2024 UBIDECO Institute, Switzerland
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#[cfg(not(any(feature = "base64", feature = "base85")))]
compile_error!("either base64 or base85 feature must be specified");

#[cfg(all(feature = "base64", feature = "base85"))]
compile_error!("either base64 or base85 feature must be specified");

#[macro_use]
extern crate amplify;

use core::fmt::{self, Display, Formatter};
use core::str::FromStr;
use std::fmt::Debug;

#[cfg(feature = "strict")]
use amplify::confinement::U24 as U24MAX;
use amplify::confinement::{self, Confined};
use amplify::num::u24;
use amplify::{hex, Bytes32};
use sha2::{Digest, Sha256};
#[cfg(feature = "strict")]
use strict_encoding::{StrictDeserialize, StrictSerialize};

pub const ASCII_ARMOR_MAX_LEN: usize = u24::MAX.to_usize();
pub const ASCII_ARMOR_ID: &'static str = "Id";
pub const ASCII_ARMOR_CHECKSUM_SHA256: &'static str = "Check-SHA256";

pub struct DisplayAsciiArmored<'a, A: AsciiArmor>(&'a A);

impl<'a, A: AsciiArmor> DisplayAsciiArmored<'a, A> {
    fn data_digest(&self) -> (Vec<u8>, Option<Bytes32>) {
        let data = self.0.to_ascii_armored_data();
        let digest = Sha256::digest(&data);
        (data, Some(Bytes32::from_byte_array(digest)))
    }
}

impl<'a, A: AsciiArmor> Display for DisplayAsciiArmored<'a, A> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "-----BEGIN {}-----", A::PLATE_TITLE)?;

        let (data, digest) = self.data_digest();
        for header in self.0.ascii_armored_headers() {
            writeln!(f, "{header}")?;
        }
        if let Some(digest) = digest {
            writeln!(f, "{ASCII_ARMOR_CHECKSUM_SHA256}: {digest}")?;
        }
        writeln!(f)?;

        #[cfg(feature = "base85")]
        let data = base85::encode(&data);
        #[cfg(feature = "base64")]
        let data = {
            use base64::Engine;
            base64::prelude::BASE64_STANDARD.encode(&data)
        };
        let mut data = data.as_str();
        while data.len() >= 80 {
            let (line, rest) = data.split_at(80);
            writeln!(f, "{}", line)?;
            data = rest;
        }
        writeln!(f, "{}", data)?;

        writeln!(f, "\n-----END {}-----", A::PLATE_TITLE)?;

        Ok(())
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArmorHeader {
    pub title: String,
    pub values: Vec<String>,
    pub params: Vec<(String, String)>,
}

impl ArmorHeader {
    pub fn new(title: &'static str, value: String) -> Self {
        ArmorHeader {
            title: title.to_owned(),
            values: vec![value],
            params: none!(),
        }
    }
    pub fn with(title: &'static str, values: impl IntoIterator<Item = String>) -> Self {
        ArmorHeader {
            title: title.to_owned(),
            values: values.into_iter().collect(),
            params: none!(),
        }
    }
}

impl Display for ArmorHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let pfx = if self.values.len() == 1 { " " } else { "\n\t" };
        write!(f, "{}:{}", self.title, pfx)?;
        for (i, val) in self.values.iter().enumerate() {
            if i > 0 {
                f.write_str(",\n\t")?;
            }
            write!(f, "{}", val)?;
        }

        for (name, val) in &self.params {
            write!(f, ";\n\t{name}={val}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Eq, PartialEq, Debug, Display, Error, From)]
#[display(doc_comments)]
pub enum ArmorParseError {
    /// armored header has invalid format ("{0}").
    InvalidHeaderFormat(String),

    /// armored header '{0}' has invalid parameter '{1}'.
    InvalidHeaderParam(String, String),

    /// the provided text doesn't represent a recognizable ASCII-armored RGB
    /// bindle encoding.
    WrongStructure,

    /// ASCII armor data has invalid Base85 encoding.
    Base85,

    /// ASCII armor data has invalid Base64 encoding.
    Base64,

    /// header providing checksum for the armored data must not contain additional
    /// parameters.
    NonEmptyChecksumParams,

    /// multiple checksum headers provided.
    MultipleChecksums,

    /// ASCII armor contains unparsable checksum. Details: {0}
    #[from]
    UnparsableChecksum(hex::Error),

    /// ASCII armor checksum doesn't match the actual data.
    MismatchedChecksum,

    /// unrecognized header '{0}' in ASCII armor.
    UnrecognizedHeader(String),
}

impl FromStr for ArmorHeader {
    type Err = ArmorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (title, rest) =
            s.split_once(':').ok_or_else(|| ArmorParseError::InvalidHeaderFormat(s.to_owned()))?;
        let rest = rest.trim();
        let mut split = rest.split(';');
        let values =
            split.next().ok_or_else(|| ArmorParseError::InvalidHeaderFormat(s.to_owned()))?.trim();
        let mut params = vec![];
        for param in split {
            let (name, val) = s.split_once('=').ok_or_else(|| {
                ArmorParseError::InvalidHeaderParam(title.to_owned(), param.to_owned())
            })?;
            params.push((name.trim().to_owned(), val.trim().to_owned()));
        }
        let values = values.split(',').map(|val| val.trim().to_owned()).collect();
        Ok(ArmorHeader {
            title: title.to_owned(),
            values,
            params,
        })
    }
}

pub trait AsciiArmor: Sized {
    type Err: Debug + From<ArmorParseError>;

    const PLATE_TITLE: &'static str;

    fn to_ascii_armored_string(&self) -> String { format!("{}", self.display_ascii_armored()) }
    fn display_ascii_armored(&self) -> DisplayAsciiArmored<Self> { DisplayAsciiArmored(self) }
    fn ascii_armored_headers(&self) -> Vec<ArmorHeader> { none!() }
    fn ascii_armored_digest(&self) -> Option<Bytes32> { DisplayAsciiArmored(self).data_digest().1 }
    fn to_ascii_armored_data(&self) -> Vec<u8>;

    fn from_ascii_armored_str(s: &str) -> Result<Self, Self::Err> {
        let mut lines = s.lines();
        let first = format!("-----BEGIN {}-----", Self::PLATE_TITLE);
        let last = format!("-----END {}-----", Self::PLATE_TITLE);
        if (lines.next(), lines.next_back()) != (Some(&first), Some(&last)) {
            return Err(ArmorParseError::WrongStructure.into());
        }
        let mut checksum = None;
        let mut headers = vec![];
        for line in lines.by_ref() {
            if line.is_empty() {
                break;
            }
            let header = ArmorHeader::from_str(&line)?;
            if header.title == ASCII_ARMOR_CHECKSUM_SHA256 {
                if !header.params.is_empty() {
                    return Err(ArmorParseError::NonEmptyChecksumParams.into());
                }
                if header.values.is_empty() || header.values.len() > 1 {
                    return Err(ArmorParseError::MultipleChecksums.into());
                }
                checksum = Some(header.values[0].to_owned());
            } else {
                headers.push(header);
            }
        }
        let armor = lines.collect::<String>();
        #[cfg(feature = "base85")]
        let data = base85::decode(&armor).map_err(|_| ArmorParseError::Base85)?;
        #[cfg(feature = "base64")]
        let data = {
            use base64::Engine;
            base64::prelude::BASE64_STANDARD.decode(&armor).map_err(|_| ArmorParseError::Base64)?
        };
        if let Some(checksum) = checksum {
            let checksum = Bytes32::from_str(&checksum)
                .map_err(|err| ArmorParseError::UnparsableChecksum(err))?;
            let expected = Bytes32::from_byte_array(Sha256::digest(&data));
            if checksum != expected {
                return Err(ArmorParseError::MismatchedChecksum.into());
            }
        }
        let me = Self::with_headers_data(headers, data)?;
        Ok(me)
    }

    fn with_headers_data(headers: Vec<ArmorHeader>, data: Vec<u8>) -> Result<Self, Self::Err>;
}

#[cfg(feature = "strict")]
#[derive(Debug, Display, Error, From)]
#[display(doc_comments)]
pub enum StrictArmorError {
    /// ASCII armor misses required Id header.
    MissedId,

    /// multiple Id headers.
    MultipleIds,

    #[cfg(feature = "baid64")]
    /// Id header of the ASCII armor contains unparsable information. Details: {0}
    #[from]
    InvalidId(baid64::Baid64ParseError),

    /// the actual ASCII armor doesn't match the provided id.
    ///
    /// Actual id: {actual}.
    ///
    /// Expected id: {expected}.
    MismatchedId { actual: String, expected: String },

    /// unable to decode the provided ASCII armor. Details: {0}
    #[from]
    Deserialize(strict_encoding::DeserializeError),

    /// ASCII armor contains more than 16MB of data.
    #[from(confinement::Error)]
    TooLarge,

    #[from]
    #[display(inner)]
    Armor(ArmorParseError),
}

#[cfg(feature = "strict")]
pub trait StrictArmor: StrictSerialize + StrictDeserialize {
    type Id: Copy + Eq + Debug + Display + FromStr<Err = baid64::Baid64ParseError>;

    const PLATE_TITLE: &'static str;

    fn armor_id(&self) -> Self::Id;
    fn checksum_armor(&self) -> bool { false }
    fn armor_headers(&self) -> Vec<ArmorHeader> { none!() }
    fn parse_armor_headers(&mut self, _headers: Vec<ArmorHeader>) -> Result<(), StrictArmorError> {
        Ok(())
    }
}

#[cfg(feature = "strict")]
impl<T> AsciiArmor for T
where T: StrictArmor
{
    type Err = StrictArmorError;
    const PLATE_TITLE: &'static str = <T as StrictArmor>::PLATE_TITLE;

    fn ascii_armored_headers(&self) -> Vec<ArmorHeader> {
        let mut headers = vec![ArmorHeader::new(ASCII_ARMOR_ID, self.armor_id().to_string())];
        headers.extend(self.armor_headers());
        headers
    }

    fn to_ascii_armored_data(&self) -> Vec<u8> {
        self.to_strict_serialized::<U24MAX>()
            .expect("data too large for ASCII armoring")
            .into_inner()
    }

    fn with_headers_data(headers: Vec<ArmorHeader>, data: Vec<u8>) -> Result<Self, Self::Err> {
        let id =
            headers.iter().find(|h| h.title == ASCII_ARMOR_ID).ok_or(StrictArmorError::MissedId)?;
        // TODO: Proceed and check id
        if id.values.is_empty() || id.values.len() > 1 {
            return Err(StrictArmorError::MultipleIds.into());
        }
        let expected = T::Id::from_str(&id.values[0]).map_err(StrictArmorError::from)?;
        let data = Confined::try_from(data).map_err(StrictArmorError::from)?;
        let mut me =
            Self::from_strict_serialized::<U24MAX>(data).map_err(StrictArmorError::from)?;
        me.parse_armor_headers(headers)?;
        let actual = me.armor_id();
        if expected != actual {
            return Err(StrictArmorError::MismatchedId {
                expected: expected.to_string(),
                actual: actual.to_string(),
            }
            .into());
        }
        Ok(me)
    }
}

impl AsciiArmor for Vec<u8> {
    type Err = ArmorParseError;
    const PLATE_TITLE: &'static str = "DATA";

    fn to_ascii_armored_data(&self) -> Vec<u8> { self.clone() }

    fn with_headers_data(headers: Vec<ArmorHeader>, data: Vec<u8>) -> Result<Self, Self::Err> {
        assert!(headers.is_empty());
        Ok(data)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn roundtrip() {
        let noise = Sha256::digest("some test data");
        let data = noise.as_slice().repeat(100).iter().cloned().collect::<Vec<u8>>();
        let armor = data.to_ascii_armored_string();
        let data2 = Vec::<u8>::from_ascii_armored_str(&armor).unwrap();
        let armor2 = data2.to_ascii_armored_string();
        assert_eq!(data, data2);
        assert_eq!(armor, armor2);
    }
}
