use std::sync::Arc;
#[cfg(feature = "dev-context-only-utils")]
use qualifier_attr::qualifiers;
use solana_account::{ReadableAccount, WritableAccount};
use {
    crate::{IndexOfAccount, MAX_ACCOUNT_DATA_GROWTH_PER_TRANSACTION, MAX_ACCOUNT_DATA_LEN},
    solana_account::AccountSharedData,
    solana_instruction::error::InstructionError,
    solana_pubkey::Pubkey,
    std::{
        cell::{Cell, UnsafeCell},
        ops::{Deref, DerefMut},
    },
};

#[repr(C)]
#[derive(Debug)]
struct SVMAccount {
    key: Pubkey,
    owner: Pubkey,
    lamports: u64,
    // The payload is going to be filled with the guest virtual address of the account payload
    // vector.
    // payload: VmSlice,
}

#[derive(Debug)]
struct SVMAccountDeprecated {
    rent_epoch: u64,
    executable: bool,
}

struct PrivateAccountFields {
    deprecated_fields: SVMAccountDeprecated,
    payload: Arc<Vec<u8>>,
}

struct TransactionAccountView<'a> {
    svm_account: &'a SVMAccount,
    deprecated_fields: &'a SVMAccountDeprecated,
    payload: &'a [u8],
}

impl ReadableAccount for TransactionAccountView<'_> {
    fn lamports(&self) -> u64 {
        self.svm_account.lamports
    }

    fn data(&self) -> &[u8] {
        self.payload
    }

    fn owner(&self) -> &Pubkey {
        &self.svm_account.owner
    }

    fn executable(&self) -> bool {
        self.deprecated_fields.executable
    }

    fn rent_epoch(&self) -> u64 {
        self.deprecated_fields.rent_epoch
    }
}

#[derive(Debug)]
struct TransactionAccountMutView<'a> {
    svm_account: &'a mut SVMAccount,
    deprecated_fields: &'a mut SVMAccountDeprecated,
    payload: &'a mut Arc<Vec<u8>>,
}

impl ReadableAccount for TransactionAccountMutView<'_> {
    fn lamports(&self) -> u64 {
        self.svm_account.lamports
    }

    fn data(&self) -> &[u8] {
        self.payload.as_slice()
    }

    fn owner(&self) -> &Pubkey {
        &self.svm_account.owner
    }

    fn executable(&self) -> bool {
        self.deprecated_fields.executable
    }

    fn rent_epoch(&self) -> u64 {
        self.deprecated_fields.rent_epoch
    }
}

impl WritableAccount for TransactionAccountMutView<'_> {
    fn set_lamports(&mut self, lamports: u64) {
        self.svm_account.lamports = lamports;
    }

    fn data_as_mut_slice(&mut self) -> &mut [u8] {
        Arc::make_mut(self.payload).as_mut_slice()
    }

    fn set_owner(&mut self, owner: Pubkey) {
        self.svm_account.owner = owner;
    }

    fn copy_into_owner_from_slice(&mut self, source: &[u8]) {
        self.svm_account.owner.as_mut().copy_from_slice(source);
    }

    fn set_executable(&mut self, executable: bool) {
        self.deprecated_fields.executable = executable;
    }

    fn set_rent_epoch(&mut self, epoch: u64) {
        self.deprecated_fields.rent_epoch = epoch;
    }

    fn create(_lamports: u64, _data: Vec<u8>, _owner: Pubkey, _executable: bool, _rent_epoch: u64) -> Self {
        panic!("It is not possible to create a TransactionMutView");
    }
}


/// An account key and the matching account
pub type TransactionAccount = (Pubkey, AccountSharedData);
pub(crate) type OwnedTransactionAccounts = (
    UnsafeCell<Box<[TransactionAccount]>>,
    Box<[Cell<bool>]>,
    Cell<i64>,
);

#[derive(Debug)]
pub struct TransactionAccounts {
    shared_account_metadata: UnsafeCell<Box<[SVMAccount]>>,
    private_account_fields: UnsafeCell<Box<[PrivateAccountFields]>>,
    borrow_counters: Box<[BorrowCounter]>,
    touched_flags: Box<[Cell<bool>]>,
    resize_delta: Cell<i64>,
    lamports_delta: Cell<i128>,
}

impl TransactionAccounts {
    #[cfg(not(target_os = "solana"))]
    pub(crate) fn new(accounts: Vec<TransactionAccount>) -> TransactionAccounts {
        let touched_flags = vec![Cell::new(false); accounts.len()].into_boxed_slice();
        let borrow_counters = vec![BorrowCounter::default(); accounts.len()].into_boxed_slice();
        let (shared_accounts, private_fields) = accounts.into_iter().map(|item|
            (
                SVMAccount {
                    key: item.0,
                    owner: *item.1.owner(),
                    lamports: item.1.lamports(),
                },
                PrivateAccountFields {
                    deprecated_fields: SVMAccountDeprecated {
                        rent_epoch: item.1.rent_epoch(),
                        executable: item.1.executable(),
                    },
                    payload: item.1.data_clone(),
                }
            )
        ).collect::<(Vec<SVMAccount>, Vec<PrivateAccountFields>)>();

        TransactionAccounts {
            shared_account_metadata: UnsafeCell::new(shared_accounts.into_boxed_slice()),
            private_account_fields: UnsafeCell::new(private_fields.into_boxed_slice()),
            borrow_counters,
            touched_flags,
            resize_delta: Cell::new(0),
            lamports_delta: Cell::new(0),
        }
    }

    pub(crate) fn len(&self) -> usize {
        // SAFETY: The borrow is local to this function and is only reading length.
        unsafe { (*self.shared_account_metadata.get()).len() }
    }

    #[cfg(not(target_os = "solana"))]
    pub fn touch(&self, index: IndexOfAccount) -> Result<(), InstructionError> {
        self.touched_flags
            .get(index as usize)
            .ok_or(InstructionError::MissingAccount)?
            .set(true);
        Ok(())
    }

    pub(crate) fn update_accounts_resize_delta(
        &self,
        old_len: usize,
        new_len: usize,
    ) -> Result<(), InstructionError> {
        let accounts_resize_delta = self.resize_delta.get();
        self.resize_delta.set(
            accounts_resize_delta.saturating_add((new_len as i64).saturating_sub(old_len as i64)),
        );
        Ok(())
    }

    pub(crate) fn can_data_be_resized(
        &self,
        old_len: usize,
        new_len: usize,
    ) -> Result<(), InstructionError> {
        // The new length can not exceed the maximum permitted length
        if new_len > MAX_ACCOUNT_DATA_LEN as usize {
            return Err(InstructionError::InvalidRealloc);
        }
        // The resize can not exceed the per-transaction maximum
        let length_delta = (new_len as i64).saturating_sub(old_len as i64);
        if self.resize_delta.get().saturating_add(length_delta)
            > MAX_ACCOUNT_DATA_GROWTH_PER_TRANSACTION
        {
            return Err(InstructionError::MaxAccountsDataAllocationsExceeded);
        }
        Ok(())
    }

    #[cfg_attr(feature = "dev-context-only-utils", qualifiers(pub))]
    pub(crate) fn try_borrow_mut(
        &self,
        index: IndexOfAccount,
    ) -> Result<AccountRefMut, InstructionError> {
        let borrow_counter = self
            .borrow_counters
            .get(index as usize)
            .ok_or(InstructionError::MissingAccount)?;
        borrow_counter.try_borrow_mut()?;

        // SAFETY: The borrow counter guarantees this is the only mutable borrow of this account.
        // The unwrap is safe because accounts.len() == borrow_counters.len(), so the missing
        // account error should have been returned above.
        let svm_account = unsafe { (*self.shared_account_metadata.get()).get_mut(index as usize).unwrap() };
        let private_fields = unsafe { (*self.private_account_fields.get()).get_mut(index as usize).unwrap() };

        let account = TransactionAccountMutView {
            svm_account,
            deprecated_fields: &mut private_fields.deprecated_fields,
            payload: &mut private_fields.payload
        };

        Ok(AccountRefMut {
            account,
            borrow_counter,
        })
    }

    pub fn try_borrow(&self, index: IndexOfAccount) -> Result<AccountRef, InstructionError> {
        let borrow_counter = self
            .borrow_counters
            .get(index as usize)
            .ok_or(InstructionError::MissingAccount)?;
        borrow_counter.try_borrow()?;

        // SAFETY: The borrow counter guarantees there are no mutable borrow of this account.
        // The unwrap is safe because accounts.len() == borrow_counters.len(), so the missing
        // account error should have been returned above.
        let svm_account = unsafe { (*self.shared_account_metadata.get()).get(index as usize).unwrap() };
        let private_fields = unsafe { (*self.private_account_fields.get()).get(index as usize).unwrap() };

        let account = TransactionAccountView {
            svm_account,
            deprecated_fields: &private_fields.deprecated_fields,
            payload: &private_fields.payload
        };

        Ok(AccountRef {
            account,
            borrow_counter,
        })
    }

    pub(crate) fn add_lamports_delta(&self, balance: i128) -> Result<(), InstructionError> {
        let delta = self.lamports_delta.get();
        self.lamports_delta.set(
            delta
                .checked_add(balance)
                .ok_or(InstructionError::ArithmeticOverflow)?,
        );
        Ok(())
    }

    pub(crate) fn get_lamports_delta(&self) -> i128 {
        self.lamports_delta.get()
    }

    pub(crate) fn into_account_shared_data(self) -> Vec<AccountSharedData> {
        self.shared_account_metadata.get_mut().into_iter().zip(
            self.private_account_fields.get_mut().into_iter()
        ).map(
            |(shared_fields, private_fields)|
                AccountSharedData::shared
        )
    }

    pub(crate) fn take(self) -> OwnedTransactionAccounts {
        (self.shared_account_metadata, self.touched_flags, self.resize_delta)
    }

    pub fn resize_delta(&self) -> i64 {
        self.resize_delta.get()
    }

    pub(crate) fn account_key(&self, index: IndexOfAccount) -> Option<&Pubkey> {
        // SAFETY: We never modify an account key, so returning a reference to it is safe.
        unsafe { (*self.shared_account_metadata.get()).get(index as usize).map(|acc| &acc.key) }
    }

    pub(crate) fn account_keys_iter(&self) -> impl Iterator<Item = &Pubkey> {
        // SAFETY: We never modify account keys, so returning an immutable reference to them is safe.
        unsafe { (*self.shared_account_metadata.get()).iter().map(|item| &item.key) }
    }
}

#[derive(Default, Debug, Clone)]
struct BorrowCounter {
    counter: Cell<i8>,
}

impl BorrowCounter {
    #[inline]
    fn is_writing(&self) -> bool {
        self.counter.get() < 0
    }

    #[inline]
    fn is_reading(&self) -> bool {
        self.counter.get() > 0
    }

    #[inline]
    fn try_borrow(&self) -> Result<(), InstructionError> {
        if self.is_writing() {
            return Err(InstructionError::AccountBorrowFailed);
        }

        if let Some(counter) = self.counter.get().checked_add(1) {
            self.counter.set(counter);
            return Ok(());
        }

        Err(InstructionError::AccountBorrowFailed)
    }

    #[inline]
    fn try_borrow_mut(&self) -> Result<(), InstructionError> {
        if self.is_writing() || self.is_reading() {
            return Err(InstructionError::AccountBorrowFailed);
        }

        self.counter.set(self.counter.get().saturating_sub(1));

        Ok(())
    }

    #[inline]
    fn release_borrow(&self) {
        self.counter.set(self.counter.get().saturating_sub(1));
    }

    #[inline]
    fn release_borrow_mut(&self) {
        self.counter.set(self.counter.get().saturating_add(1));
    }
}

pub struct AccountRef<'a> {
    account: TransactionAccountView<'a>,
    borrow_counter: &'a BorrowCounter,
}

impl Drop for AccountRef<'_> {
    fn drop(&mut self) {
        self.borrow_counter.release_borrow();
    }
}

impl<'a> Deref for AccountRef<'a> {
    type Target = TransactionAccountView<'a>;
    fn deref(&self) -> &Self::Target {
        &self.account
    }
}

#[derive(Debug)]
pub struct AccountRefMut<'a> {
    account: TransactionAccountMutView<'a>,
    borrow_counter: &'a BorrowCounter,
}

impl Drop for AccountRefMut<'_> {
    fn drop(&mut self) {
        self.borrow_counter.release_borrow_mut();
    }
}

impl<'a> Deref for AccountRefMut<'a> {
    type Target = TransactionAccountMutView<'a>;
    fn deref(&self) -> &Self::Target {
        &self.account
    }
}

impl<'a> DerefMut for AccountRefMut<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.account
    }
}

#[cfg(test)]
mod tests {
    use {
        crate::transaction_accounts::TransactionAccounts, solana_account::AccountSharedData,
        solana_instruction::error::InstructionError, solana_pubkey::Pubkey,
    };

    #[test]
    fn test_missing_account() {
        let accounts = vec![
            (
                Pubkey::new_unique(),
                AccountSharedData::new(2, 1, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(2, 1, &Pubkey::new_unique()),
            ),
        ];

        let tx_accounts = TransactionAccounts::new(accounts);

        let res = tx_accounts.try_borrow(3);
        assert_eq!(res.err(), Some(InstructionError::MissingAccount));

        let res = tx_accounts.try_borrow_mut(3);
        assert_eq!(res.err(), Some(InstructionError::MissingAccount));
    }

    #[test]
    fn test_invalid_borrow() {
        let accounts = vec![
            (
                Pubkey::new_unique(),
                AccountSharedData::new(2, 1, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(2, 1, &Pubkey::new_unique()),
            ),
        ];

        let tx_accounts = TransactionAccounts::new(accounts);

        // Two immutable borrows are valid
        {
            let acc_1 = tx_accounts.try_borrow(0);
            assert!(acc_1.is_ok());

            let acc_2 = tx_accounts.try_borrow(1);
            assert!(acc_2.is_ok());

            let acc_1_new = tx_accounts.try_borrow(0);
            assert!(acc_1_new.is_ok());

            assert_eq!(&*acc_1.unwrap(), &*acc_1_new.unwrap());
        }

        // Two mutable borrows are invalid
        {
            let acc_1 = tx_accounts.try_borrow_mut(0);
            assert!(acc_1.is_ok());

            let acc_2 = tx_accounts.try_borrow_mut(1);
            assert!(acc_2.is_ok());

            let acc_1_new = tx_accounts.try_borrow_mut(0);
            assert_eq!(acc_1_new.err(), Some(InstructionError::AccountBorrowFailed));
        }

        // Mutable after immutable must fail
        {
            let acc_1 = tx_accounts.try_borrow(0);
            assert!(acc_1.is_ok());

            let acc_2 = tx_accounts.try_borrow(1);
            assert!(acc_2.is_ok());

            let acc_1_new = tx_accounts.try_borrow_mut(0);
            assert_eq!(acc_1_new.err(), Some(InstructionError::AccountBorrowFailed));
        }

        // Immutable after mutable must fail
        {
            let acc_1 = tx_accounts.try_borrow_mut(0);
            assert!(acc_1.is_ok());

            let acc_2 = tx_accounts.try_borrow_mut(1);
            assert!(acc_2.is_ok());

            let acc_1_new = tx_accounts.try_borrow(0);
            assert_eq!(acc_1_new.err(), Some(InstructionError::AccountBorrowFailed));
        }

        // Different scopes are good
        {
            let acc_1 = tx_accounts.try_borrow_mut(0);
            assert!(acc_1.is_ok());
        }

        {
            let acc_1 = tx_accounts.try_borrow_mut(0);
            assert!(acc_1.is_ok());
        }
    }

    #[test]
    fn too_many_borrows() {
        let accounts = vec![
            (
                Pubkey::new_unique(),
                AccountSharedData::new(2, 1, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(2, 1, &Pubkey::new_unique()),
            ),
        ];

        let tx_accounts = TransactionAccounts::new(accounts);
        let mut borrows = Vec::new();
        for i in 0..129 {
            let acc = tx_accounts.try_borrow(1);
            if i < 127 {
                assert!(acc.is_ok());
                borrows.push(acc.unwrap());
            } else {
                assert_eq!(acc.err(), Some(InstructionError::AccountBorrowFailed));
            }
        }
    }
}
