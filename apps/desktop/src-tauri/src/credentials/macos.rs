use super::*;
use core_foundation::data::CFData;
use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass};
use security_framework::passwords::{delete_generic_password, generic_password, PasswordOptions};

#[derive(Default)]
pub(crate) struct KeychainStore;

fn status(code: i32) -> CredentialError {
    match code {
        -25300 => CredentialError::Missing,
        -25299 => CredentialError::Conflict,
        -25308 | -25291 => CredentialError::Unavailable,
        _ => CredentialError::Store,
    }
}
impl CredentialStore for KeychainStore {
    fn put(&mut self, id: &CredentialId, secret: &Secret) -> Result<(), CredentialError> {
        // SecItemAdd is insert-only. The convenience password setter would silently update.
        let value = ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(secret.expose().as_bytes()),
        };
        ItemAddOptions::new(value)
            .set_service(SERVICE)
            .set_account_name(id.canonical())
            .add()
            .map_err(|error| status(error.code()))
    }
    fn get(&self, id: &CredentialId) -> Result<Secret, CredentialError> {
        let bytes = Zeroizing::new(
            generic_password(PasswordOptions::new_generic_password(
                SERVICE,
                &id.canonical(),
            ))
            .map_err(|error| status(error.code()))?,
        );
        let value = std::str::from_utf8(&bytes).map_err(|_| CredentialError::InvalidSecret)?;
        Secret::new(value.to_owned())
    }
    fn delete(&mut self, id: &CredentialId) -> Result<(), CredentialError> {
        delete_generic_password(SERVICE, &id.canonical()).map_err(|error| status(error.code()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_errors_are_stable_categories_without_queries_or_keychain_mutation() {
        assert_eq!(status(-25300), CredentialError::Missing);
        assert_eq!(status(-25299), CredentialError::Conflict);
        assert_eq!(status(-25308), CredentialError::Unavailable);
        assert_eq!(status(-1), CredentialError::Store);
    }
}
