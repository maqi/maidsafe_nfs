// Copyright 2015 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

use std::cmp;
use std::sync::{Arc, Mutex};

use sodiumoxide::crypto::box_;

use errors::NfsError;
use file::File;
use xor_name::XorName;
use maidsafe_utilities::serialisation::{serialise, deserialise};
use metadata::directory_key::DirectoryKey;
use metadata::directory_metadata::DirectoryMetadata;
use safe_core::client::Client;
use safe_core::SelfEncryptionStorage;
use self_encryption::datamap::DataMap;
use self_encryption::SelfEncryptor;

/// DirectoryListing is the representation of a deserialised Directory in the network
#[derive(Debug, RustcEncodable, RustcDecodable, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct DirectoryListing {
    metadata       : DirectoryMetadata,
    sub_directories: Vec<DirectoryMetadata>,
    files          : Vec<File>,
}

impl DirectoryListing {
    /// Create a new DirectoryListing
    pub fn new(name           : String,
               tag_type       : u64,
               user_metadata  : Vec<u8>,
               versioned      : bool,
               access_level   : ::AccessLevel,
               parent_dir_key: Option<DirectoryKey>) -> Result<DirectoryListing, NfsError> {
        let meta_data = try!(DirectoryMetadata::new(name,
                                                    tag_type,
                                                    versioned,
                                                    access_level,
                                                    user_metadata,
                                                    parent_dir_key));
        Ok(DirectoryListing {
            metadata       : meta_data,
            sub_directories: Vec::new(),
            files          : Vec::new(),
        })
    }

    /// Returns the DirectoryKey representing the DirectoryListing
    pub fn get_key(&self) -> &DirectoryKey {
        &self.metadata.get_key()
    }

    /// Get Directory metadata
    pub fn get_metadata(&self) -> &DirectoryMetadata {
        &self.metadata
    }

    /// Get Directory metadata in mutable format so that it can also be updated
    pub fn get_mut_metadata(&mut self) -> &mut DirectoryMetadata {
        &mut self.metadata
    }

    /// Get all files in this DirectoryListing
    pub fn get_files(&self) -> &Vec<File> {
        &self.files
    }

    /// Get all files in this DirectoryListing with mutability to update the listing of files
    pub fn get_mut_files(&mut self) -> &mut Vec<File> {
        &mut self.files
    }

    /// Get all subdirectories in this DirectoryListing
    pub fn get_sub_directories(&self) -> &Vec<DirectoryMetadata> {
        &self.sub_directories
    }

    /// Get all subdirectories in this DirectoryListing with mutability to update the listing of subdirectories
    pub fn get_mut_sub_directories(&mut self) -> &mut Vec<DirectoryMetadata> {
        &mut self.sub_directories
    }

    /// Decrypts a directory listing
    pub fn decrypt(client      : Arc<Mutex<Client>>,
                   directory_id: &XorName,
                   data        : Vec<u8>) -> Result<DirectoryListing, NfsError> {
        let decrypted_data_map = try!(unwrap_result!(client.lock()).hybrid_decrypt(&data,
                Some(&DirectoryListing::generate_nonce(directory_id))));
        let datamap: DataMap = try!(deserialise(&decrypted_data_map));
        let mut se = SelfEncryptor::new(SelfEncryptionStorage::new(client.clone()), datamap);
        let length = se.len();
        debug!("Reading encrypted storage of length {:?} ...", length);
        let serialised_directory_listing = se.read(0, length);
        Ok(try!(deserialise(&serialised_directory_listing)))
    }

    /// Encrypts the directory listing
    pub fn encrypt(&self, client: Arc<Mutex<Client>>) -> Result<Vec<u8>, NfsError> {
        let serialised_data = try!(serialise(&self));
        let mut se = SelfEncryptor::new(SelfEncryptionStorage::new(client.clone()), DataMap::None);
        debug!("Writing to storage using self encryption ...");
        se.write(&serialised_data, 0);
        let datamap = se.close();
        let serialised_data_map = try!(serialise(&datamap));
        Ok(try!(unwrap_result!(client.lock()).hybrid_encrypt(&serialised_data_map,
                Some(&DirectoryListing::generate_nonce(&self.get_key().get_id())))))
    }

    /// Get DirectoryInfo of sub_directory within a DirectoryListing.
    /// Returns the Option<DirectoryInfo> for the directory_name from the DirectoryListing
    pub fn find_file(&self, file_name: &String) -> Option<&File> {
        self.get_files().iter().find(|file| *file.get_name() == *file_name)
    }

    /// Get DirectoryInfo of sub_directory within a DirectoryListing.
    /// Returns the Option<DirectoryInfo> for the directory_name from the DirectoryListing
    pub fn find_file_by_id(&self, id: &XorName) -> Option<&File> {
        self.get_files().iter().find(|file| *file.get_id() == *id)
    }

    /// Get DirectoryInfo of sub_directory within a DirectoryListing.
    /// Returns the Option<DirectoryInfo> for the directory_name from the DirectoryListing
    pub fn find_sub_directory(&self, directory_name: &String) -> Option<&DirectoryMetadata> {
        self.get_sub_directories().iter().find(|info| *info.get_name() == *directory_name)
    }

    /// Get DirectoryInfo of sub_directory within a DirectoryListing.
    /// Returns the Option<DirectoryInfo> for the directory_name from the DirectoryListing
    pub fn find_sub_directory_by_id(&self, id: &XorName) -> Option<&DirectoryMetadata> {
        self.get_sub_directories().iter().find(|info| *info.get_id() == *id)
    }

    /// If file is present in the DirectoryListing then replace it else insert it
    pub fn upsert_file(&mut self, file: File) {
        let modified_time = file.get_metadata().get_modified_time().clone();
        // TODO try using the below approach for efficiency - also try the same in upsert_sub_directory
        // if let Some(mut existing_file) = self.files.iter_mut().find(|entry| *entry.get_name() == *file.get_name()) {
        // *existing_file = file;
        if let Some(index) = self.files.iter().position(|entry| *entry.get_id() == *file.get_id()) {
            debug!("Replacing file in directory listing ...");
            let mut existing = unwrap_option!(self.files.get_mut(index),
                                              "Programming Error - Report this as a Bug.");
            *existing = file;
        } else {
            debug!("Adding file to directory listing ...");
            self.files.push(file);
        }
        self.get_mut_metadata().set_modified_time(modified_time)
    }

    /// If DirectoryMetadata is present in the sub_directories of DirectoryListing then replace it else insert it
    pub fn upsert_sub_directory(&mut self, directory_metadata: DirectoryMetadata) {
        let modified_time = directory_metadata.get_modified_time().clone();
        if let Some(index) = self.sub_directories.iter().position(|entry| *entry.get_key().get_id() == *directory_metadata.get_key().get_id()) {
            debug!("Replacing directory listing metadata ...");
            let mut existing = unwrap_option!(self.sub_directories.get_mut(index),
                                              "Programming Error - Report this as a Bug.");
            *existing = directory_metadata;
        } else {
            debug!("Adding metadata to directory listing ...");
            self.sub_directories.push(directory_metadata);
        }
        self.get_mut_metadata().set_modified_time(modified_time);
    }

    /// Remove a sub_directory
    pub fn remove_sub_directory(&mut self, directory_name: &String) -> Result<(), NfsError> {
        let index = try!(self.get_sub_directories().iter().position(|dir_info| *dir_info.get_name() == *directory_name).ok_or(NfsError::DirectoryNotFound));
        debug!("Removing sub directory at index {:?} ...", index);
        let _ = self.get_mut_sub_directories().remove(index);
        Ok(())
    }

    /// Remove a file
    pub fn remove_file(&mut self, file_name: &String) -> Result<(), NfsError> {
        let index = try!(self.get_files().iter().position(|file| *file.get_name() == *file_name).ok_or(NfsError::FileNotFound));
        debug!("Removing file at index {:?} ...", index);
        let _ = self.get_mut_files().remove(index);
        Ok(())
    }

    /// Generates a nonce based on the directory_id
    pub fn generate_nonce(directory_id: &XorName) -> box_::Nonce {
        let mut nonce = [0u8; box_::NONCEBYTES];
        let min_length = cmp::min(nonce.len(), directory_id.0.len());
        for i in 0..min_length {
            nonce[i] = directory_id.0[i];
        }
        box_::Nonce(nonce)
    }
}

#[cfg(test)]
mod test {
    use std::sync::{Arc, Mutex};
    use super::DirectoryListing;
    use file::File;
    use maidsafe_utilities::serialisation::{serialise, deserialise};
    use metadata::file_metadata::FileMetadata;
    use safe_core::utility::test_utils;
    use self_encryption::datamap::DataMap;

    #[test]
    fn serialise_and_deserialise_directory_listing() {
        let obj_before = unwrap_result!(DirectoryListing::new("Home".to_string(),
                                                            10,
                                                            "some metadata about the directory".to_string().into_bytes(),
                                                            true,
                                                            ::AccessLevel::Private,
                                                            None));

        let serialised_data = unwrap_result!(serialise(&obj_before));
        let obj_after = unwrap_result!(deserialise(&serialised_data));
        assert_eq!(obj_before, obj_after);
    }

    #[test]
    fn encrypt_and_decrypt_directory_listing() {
        let test_client = unwrap_result!(test_utils::get_client());
        let client = Arc::new(Mutex::new(test_client));
        let directory_listing = unwrap_result!(DirectoryListing::new("Home".to_string(),
                                                                     10,
                                                                     Vec::new(),
                                                                     true,
                                                                     ::AccessLevel::Private,
                                                                     None));
        let encrypted_data = unwrap_result!(directory_listing.encrypt(client.clone()));
        let decrypted_listing = unwrap_result!(DirectoryListing::decrypt(client.clone(),
                                                                       directory_listing.get_key().get_id(),
                                                                       encrypted_data));
        assert_eq!(directory_listing, decrypted_listing);
    }

    #[test]
    fn find_upsert_remove_file() {
        let mut directory_listing = unwrap_result!(DirectoryListing::new("Home".to_string(),
                                                                         10,
                                                                         Vec::new(),
                                                                         true,
                                                                         ::AccessLevel::Private,
                                                                         None));
        let mut file = unwrap_result!(File::new(FileMetadata::new("index.html".to_string(), Vec::new()),
                                                DataMap::None));
        assert!(directory_listing.find_file(file.get_name()).is_none());
        directory_listing.upsert_file(file.clone());
        assert!(directory_listing.find_file(file.get_name()).is_some());

        file.get_mut_metadata().set_name("home.html".to_string());
        directory_listing.upsert_file(file.clone());
        assert_eq!(directory_listing.get_files().len(), 1);
        let file2 = unwrap_result!(File::new(FileMetadata::new("demo.html".to_string(), Vec::new()),
                                             DataMap::None));
        directory_listing.upsert_file(file2.clone());
        assert_eq!(directory_listing.get_files().len(), 2);

        let _ = unwrap_option!(directory_listing.find_file(file.get_name()), "File not found");
        let _ = unwrap_option!(directory_listing.find_file(file2.get_name()), "File not found");

        let _ = unwrap_result!(directory_listing.remove_file(file.get_metadata().get_name()));
        assert!(directory_listing.find_file(file.get_name()).is_none());
        assert!(directory_listing.find_file(file2.get_name()).is_some());
        assert_eq!(directory_listing.get_files().len(), 1);

        let _ = unwrap_result!(directory_listing.remove_file(file2.get_metadata().get_name()));
        assert_eq!(directory_listing.get_files().len(), 0);
    }

    #[test]
    fn find_upsert_remove_directory() {
        let mut directory_listing = unwrap_result!(DirectoryListing::new("Home".to_string(),
                                                                         10,
                                                                         Vec::new(),
                                                                         true,
                                                                         ::AccessLevel::Private,
                                                                         None));
        let mut sub_directory = unwrap_result!(DirectoryListing::new("Child one".to_string(),
                                                                     10,
                                                                     Vec::new(),
                                                                     true,
                                                                     ::AccessLevel::Private,
                                                                     None));
        assert!(directory_listing.find_sub_directory(sub_directory.get_metadata().get_name()).is_none());
        directory_listing.upsert_sub_directory(sub_directory.get_metadata().clone());
        assert!(directory_listing.find_sub_directory(sub_directory.get_metadata().get_name()).is_some());

        sub_directory.get_mut_metadata().set_name("Child_1".to_string());
        directory_listing.upsert_sub_directory(sub_directory.get_metadata().clone());
        assert_eq!(directory_listing.get_sub_directories().len(), 1);
        let sub_directory_two = unwrap_result!(DirectoryListing::new("Child Two".to_string(),
                                                                     10,
                                                                     Vec::new(),
                                                                     true,
                                                                     ::AccessLevel::Private,
                                                                     None));
        directory_listing.upsert_sub_directory(sub_directory_two.get_metadata().clone());
        assert_eq!(directory_listing.get_sub_directories().len(), 2);

        let _ = unwrap_option!(directory_listing.find_sub_directory(sub_directory.get_metadata().get_name()), "Directory not found");
        let _ = unwrap_option!(directory_listing.find_sub_directory(sub_directory_two.get_metadata().get_name()), "Directory not found");

        let _ = unwrap_result!(directory_listing.remove_sub_directory(sub_directory.get_metadata().get_name()));
        assert!(directory_listing.find_sub_directory(sub_directory.get_metadata().get_name()).is_none());
        assert!(directory_listing.find_sub_directory(sub_directory_two.get_metadata().get_name()).is_some());
        assert_eq!(directory_listing.get_sub_directories().len(), 1);

        let _ = unwrap_result!(directory_listing.remove_sub_directory(sub_directory_two.get_metadata().get_name()));
        assert_eq!(directory_listing.get_sub_directories().len(), 0);
    }

}
