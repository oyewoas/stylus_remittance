#![cfg(test)]

extern crate alloc;
use alloc::{vec};
use core::cell::RefCell;
use alloc::collections::BTreeMap;

use stylus_sdk::{alloy_primitives::{address, Address, U256}, testing::*};
use remittance_protocol::{UniversalRemittance, RemittanceErrors}; // adjust path if needed

// -----------------------------
// Mock ERC20 and improved in-memory token registry (test-only)
// -----------------------------
#[derive(Default, Debug, Clone)]
struct MockERC20 {
    balances: BTreeMap<Address, U256>,
    allowances: BTreeMap<(Address, Address), U256>,
    self_addr: Address,
}
impl MockERC20 {
    pub fn deployed_at(self_addr: Address) -> Self {
        Self {
            self_addr,
            ..Default::default()
        }
    }
    pub fn mint(&mut self, to: Address, amount: U256) {
        let entry = self.balances.entry(to).or_insert(U256::ZERO);
        *entry = *entry + amount;
    }
    pub fn approve(&mut self, owner: Address, spender: Address, amount: U256) {
        self.allowances.insert((owner, spender), amount);
    }
    pub fn balance_of(&self, acct: Address) -> U256 {
        *self.balances.get(&acct).unwrap_or(&U256::ZERO)
    }
    #[allow(dead_code)]
    pub fn transfer(&mut self, from: Address, to: Address, amount: U256) -> bool {
        let fb = self.balance_of(from);
        if fb < amount {
            return false;
        }
        self.balances.insert(from, fb - amount);
        let tb = self.balance_of(to);
        self.balances.insert(to, tb + amount);
        true
    }
    #[allow(dead_code)]
    pub fn transfer_from(&mut self, spender: Address, owner: Address, to: Address, amount: U256) -> bool {
        let allow = *self.allowances.get(&(owner, spender)).unwrap_or(&U256::ZERO);
        if allow < amount {
            return false;
        }
        if !self.transfer(owner, to, amount) {
            return false;
        }
        self.allowances.insert((owner, spender), allow - amount);
        true
    }
}

// Single-layer RefCell map (avoids nested RefCell borrow complexity)
thread_local! {
    static TOKENS: RefCell<BTreeMap<Address, MockERC20>> = RefCell::new(BTreeMap::new());
}

/// Insert the mock token into registry and return its address.
fn put_token(token: MockERC20) -> Address {
    let addr = token.self_addr;
    TOKENS.with(|m| {
        let mut map = m.borrow_mut();
        map.insert(addr, token);
    });
    addr
}
/// Seed a token balance and approve for a spender. Safe if token wasn't present before.
fn seed_token_balance_and_approve(token: Address, owner: Address, spender: Address, amount: U256) {
    TOKENS.with(|m| {
        let mut map = m.borrow_mut();
        let t = map.entry(token).or_insert_with(|| MockERC20::deployed_at(token));
        t.mint(owner, amount);
        t.approve(owner, spender, amount);
    });
}

// Helper function to encode ERC20 function calls
fn encode_transfer_from(from: Address, to: Address, amount: U256) -> Vec<u8> {
    // transferFrom(address,address,uint256) selector: 0x23b872dd
    let mut data = vec![0x23, 0x87, 0x2d, 0xd];
    data.extend_from_slice(&from.as_slice());
    data.extend_from_slice(&[0u8; 12]); // padding
    data.extend_from_slice(&to.as_slice());
    data.extend_from_slice(&[0u8; 12]); // padding
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    data
}

fn encode_transfer(to: Address, amount: U256) -> Vec<u8> {
    // transfer(address,uint256) selector: 0xa9059cbb
    let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
    data.extend_from_slice(&to.as_slice());
    data.extend_from_slice(&[0u8; 12]); // padding
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    data
}

fn encode_balance_of(account: Address) -> Vec<u8> {
    // balanceOf(address) selector: 0x70a08231
    let mut data = vec![0x70, 0xa0, 0x82, 0x31];
    data.extend_from_slice(&account.as_slice());
    data.extend_from_slice(&[0u8; 12]); // padding
    data
}

// Helper to encode boolean return value (true)
fn encode_bool_true() -> Vec<u8> {
    let mut result = vec![0u8; 32];
    result[31] = 1; // true in the last byte
    result
}
// Helper to encode uint256 return value
fn encode_uint256(value: U256) -> Vec<u8> {
    value.to_be_bytes::<32>().to_vec()
}

// Helpers to simulate IERC20 behavior used by the contract
// In the Stylus test environment the contract will call out to the token address.
// We'll intercept those calls by providing functions tests call directly to mutate the registry
// so the contract's expected state matches test assumptions.
// (These helpers are used to seed and assert; the contract uses its IERC20 external calls
// which - for stylus tests - are simulated by the test framework; we ensure balances/allowances are set correctly.)

// -----------------------------
// Tests
// -----------------------------
#[test]
fn constructor_and_defaults() {
    // prepare VM & contract
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    // owner & treasury
    let owner = address!("0x1000000000000000000000000000000000000001");
    let treasury = address!("0x2000000000000000000000000000000000000002");

    vm.set_sender(owner);
    c.constructor(treasury).unwrap();

    let (payment_count, exec_count, fee_bps, paused, tre) = c.get_contract_stats();
    assert_eq!(payment_count, U256::ZERO);
    assert_eq!(exec_count, U256::ZERO);
    assert_eq!(fee_bps, U256::from(50u64)); // 0.5%
    assert!(!paused);
    assert_eq!(tre, treasury);

    // supported tokens from constructor should be present
    let usdc = address!("af88d065e77c8cC2239327C5EDb3A432268e5831");
    let usdt = address!("Fd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9");
    assert!(c.is_token_supported(usdc));
    assert!(c.is_token_supported(usdt));
}

#[test]
fn user_registration_and_double_register() {
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    let owner = address!("0x1000000000000000000000000000000000000001");
    vm.set_sender(owner);
    c.constructor(address!("0x2000000000000000000000000000000000000002")).unwrap();

    let alice = address!("0xA11CE00000000000000000000000000000000000");

    vm.set_block_timestamp(1_600_000_000); // set known timestamp for registration time check

    // register alice
    vm.set_sender(alice);
    c.register_user("Alice".into(), "NG".into(), "08030001111".into()).unwrap();

    let (name, country, phone, is_active, total_sent, total_rec, reg_time) = c.get_user_profile(alice);
    assert_eq!(name, "Alice");
    assert_eq!(country, "NG");
    assert_eq!(phone, "08030001111");
    assert!(is_active);
    assert_eq!(total_sent, U256::ZERO);
    assert_eq!(total_rec, U256::ZERO);
    assert!(reg_time > U256::ZERO);

    // registering again -> UserAlreadyRegistered
    vm.set_sender(alice);
    let err = c.register_user("Alice".into(), "NG".into(), "08030001111".into()).unwrap_err();
    match err {
        RemittanceErrors::UserAlreadyRegistered(_) => {}
        _ => panic!("expected UserAlreadyRegistered"),
    }
}

#[test]
// fn deposit_withdraw_flow_and_insufficient_balance() {
//     let vm = TestVM::default();
//     let mut c = UniversalRemittance::from(&vm);

//     let owner = address!("0x1000000000000000000000000000000000000001");
//     let treasury = address!("0x2000000000000000000000000000000000000002");
//     vm.set_sender(owner);
//     c.constructor(treasury).unwrap();

//     let alice = address!("0xA11CE00000000000000000000000000000000000");

//     vm.set_sender(alice);
//     c.register_user("Alice".into(), "NG".into(), "0803".into()).unwrap();

//     // create a mock token and put in registry
//     let token = put_token(MockERC20::deployed_at(address!("0xAAA0000000000000000000000000000000000000")));

//     // Before deposit: token not supported.
//     vm.set_sender(alice);
//     let err = c.deposit_balance(token, U256::from(100u64)).unwrap_err();

//     match err {
//         RemittanceErrors::NotSupportedToken(_) => {} // possible if token not supported — but we did add
//         _ => panic!("expected NotSupportedToken"),
//     }
//     // Add support for token
//     vm.set_sender(owner);
//     c.add_supported_token(token).unwrap();

//     // Still should fail because alice has no balance/allowance
//     vm.set_sender(alice);
//     let err = c.deposit_balance(token, U256::from(100u64)).unwrap_err();
//     println!("Deposit error as expected: {:?}", err);
//     match err {
//         RemittanceErrors::TransferFailed(_) => {}
//         _ => panic!("expected TransferFailed"),
//     }

//     // Seed mock token balance + approve for contract address
//     let contract_addr  = c.vm().contract_address();

//     seed_token_balance_and_approve(token, alice, contract_addr, U256::from(1_000u64));

//     c.deposit_balance(token, U256::from(500u64)).unwrap();
//     assert_eq!(c.get_user_balance(alice, token), U256::from(500u64));

//     let err = c.withdraw_balance(token, U256::from(600u64)).unwrap_err();
//     match err {
//         RemittanceErrors::InsufficientBalance(_) => {}
//         _ => panic!("expected InsufficientBalance"),
//     }

//     // Seed contract's mock token balance so transfers out succeed
//     TOKENS.with(|m| {
//         let mut map = m.borrow_mut();
//         let t = map.get_mut(&token).unwrap();
//         // make contract have tokens so transfer to alice on withdrawal succeeds
//         t.mint(contract_addr, U256::from(1_000u64));
//     });

//     c.withdraw_balance(token, U256::from(200u64)).unwrap();
//     assert_eq!(c.get_user_balance(alice, token), U256::from(300u64));
// }

fn deposit_withdraw_flow_and_insufficient_balance() {
    let owner = address!("0x1000000000000000000000000000000000000001");
    let treasury = address!("0x2000000000000000000000000000000000000002");
    let alice = address!("0xA11CE00000000000000000000000000000000000");
    let token = address!("0xAAA0000000000000000000000000000000000000");
    
   let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);
    let contract_addr = c.vm().contract_address();

    vm.set_sender(owner);
    c.constructor(treasury).unwrap();

    vm.set_sender(alice);
    c.register_user("Alice".into(), "NG".into(), "0803".into()).unwrap();

    // Before deposit: token not supported.
    let err = c.deposit_balance(token, U256::from(100u64)).unwrap_err();
    match err {
        RemittanceErrors::NotSupportedToken(_) => {}
        _ => panic!("expected NotSupportedToken"),
    }

    // Add support for token
    vm.set_sender(owner);
    c.add_supported_token(token).unwrap();

    // Mock failed transferFrom (insufficient balance/allowance)
    vm.set_sender(alice);
    vm.mock_call(
        token,
        encode_transfer_from(alice, contract_addr, U256::from(100u64)),
        Ok(vec![0; 32]) // return false
    );

    let err = c.deposit_balance(token, U256::from(100u64)).unwrap_err();
    match err {
        RemittanceErrors::TransferFailed(_) => {}
        _ => panic!("expected TransferFailed, got {:?}", err),
    }

    // Mock successful transferFrom for deposit
    vm.mock_call(
        token,
        encode_transfer_from(alice, contract_addr, U256::from(500u64)),
        Ok(encode_bool_true())
    );

    c.deposit_balance(token, U256::from(500u64)).unwrap();
    assert_eq!(c.get_user_balance(alice, token), U256::from(500u64));

    // Try to withdraw more than balance
    let err = c.withdraw_balance(token, U256::from(600u64)).unwrap_err();
    match err {
        RemittanceErrors::InsufficientBalance(_) => {}
        _ => panic!("expected InsufficientBalance"),
    }

    // Mock successful transfer for withdrawal
    vm.mock_call(
        token,
        encode_transfer(alice, U256::from(200u64)),
        Ok(encode_bool_true())
    );

    c.withdraw_balance(token, U256::from(200u64)).unwrap();
    assert_eq!(c.get_user_balance(alice, token), U256::from(300u64));
}

#[test]
fn manual_payment_happy_and_fee_flow() {
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    let owner = address!("0x1000000000000000000000000000000000000001");
    let treasury = address!("0x2000000000000000000000000000000000000002");
    vm.set_sender(owner);
    c.constructor(treasury).unwrap();

    let alice = address!("0xA11CE00000000000000000000000000000000000");
    let bob = address!("0xB0B0000000000000000000000000000000000000");
    vm.set_sender(alice);
    c.register_user("Alice".into(), "NG".into(), "0803".into()).unwrap();
    vm.set_sender(bob);
    c.register_user("Bob".into(), "US".into(), "000".into()).unwrap();

    // token & support
    let token = put_token(MockERC20::deployed_at(address!("0xBBB0000000000000000000000000000000000000")));
    vm.set_sender(owner);
    c.add_supported_token(token).unwrap();

    // Seed alice balance and approve contract
let contract_addr = c.vm().contract_address();

    seed_token_balance_and_approve(token, alice, contract_addr, U256::from(1_000u64));

    // ensure contract has no tokens initially (we'll let transferFrom move tokens into it via mock)
    TOKENS.with(|m| {
        let map = m.borrow();
        let t = map.get(&token).unwrap();
        // sanity checks
        assert_eq!(t.balance_of(alice), U256::from(1_000u64));
    });

    // Also ensure contract has tokens to transfer net amount and fee out when send_payment executes.
    // The contract implementation does transfer_from(sender -> contract) then transfer(contract->recipient), transfer(contract->treasury)
    // Our mock transfer_from moves tokens from alice to contract and reduces allowance.
    // So once transfer_from runs, the contract will have tokens to do the subsequent transfers.

    // Call send_payment as alice
    vm.set_sender(alice);
    c.send_payment(bob, U256::from(100u64), token, "Rent".into()).unwrap();

    // Payment record at id 0
    let (sender, recipient, amount, tok, _ts, payment_type, note, completed) = c.get_payment(U256::ZERO).unwrap();
    assert_eq!(sender, alice);
    assert_eq!(recipient, bob);
    assert_eq!(amount, U256::from(100u64));
    assert_eq!(tok, token);
    assert_eq!(payment_type, U256::ZERO); // manual
    assert_eq!(note, "Rent");
    assert!(completed);

    // Treasury should have received fee (fee 0.5% of 100 = 0 (floor), but logic uses integer math).
    // For larger amount test fee distribution: send 10_000 (1% = 100) etc.
    // We'll do another payment to verify fee movement with bigger amount

    // Seed more allowance/balance so next call works
    seed_token_balance_and_approve(token, alice, contract_addr, U256::from(10_000u64));
    vm.set_sender(alice);
    c.send_payment(bob, U256::from(10_000u64), token, "Invoice".into()).unwrap();

    // Now check balances in mock to ensure net + fee moved correctly
    TOKENS.with(|m| {
        let map = m.borrow();
        let t = map.get(&token).unwrap();
        // Contract address should have less net because transfers moved out
        // The cumulative effects: initial mint 1000 then transferFrom 100 + 10000; contract then forwarded net and fee.
        // Check recipient (bob) received net from both payments:
        // First payment net = 100 - fee(0) = 100; second net = 10000 - fee (10000 * 50 / 10000 = 50) => 9950
        let bob_balance = t.balance_of(bob);
        assert!(bob_balance >= U256::from(100u64)); // at least the first payment
        // treasury should have fee: 50
        let treasury_balance = t.balance_of(treasury);
        assert!(treasury_balance >= U256::from(50u64));
    });
}

#[test]
fn beneficiary_add_update_remove_and_get_pending_estimate() {
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    let owner = address!("0x1000000000000000000000000000000000000001");
    let treasury = address!("0x2000000000000000000000000000000000000002");
    vm.set_sender(owner);
    c.constructor(treasury).unwrap();

    let alice = address!("0xA11CE00000000000000000000000000000000000");
    let bob = address!("0xB0B0000000000000000000000000000000000000");
    vm.set_sender(alice);
    c.register_user("Alice".into(), "NG".into(), "0803".into()).unwrap();
    vm.set_sender(bob);
    c.register_user("Bob".into(), "GH".into(), "000".into()).unwrap();

    let token = put_token(MockERC20::deployed_at(address!("0xCCC0000000000000000000000000000000000000")));
    vm.set_sender(owner);
    c.add_supported_token(token).unwrap();

    // Add beneficiary for alice
    vm.set_sender(alice);
    c.add_beneficiary(bob, "Bob".into(), "friend".into(), U256::from(200u64), token, U256::from(7u64)).unwrap();

    let (addr, name, rel, amount, tok, freq, last_payment, active, total_sent) = c.get_beneficiary(alice, U256::ZERO).unwrap();
    assert_eq!(addr, bob);
    assert_eq!(name, "Bob");
    assert_eq!(rel, "friend");
    assert_eq!(amount, U256::from(200u64));
    assert_eq!(tok, token);
    assert_eq!(freq, U256::from(7u64));
    assert_eq!(last_payment, U256::ZERO);
    assert!(active);
    assert_eq!(total_sent, U256::ZERO);

    // Update beneficiary
    vm.set_sender(alice);
    c.update_beneficiary(U256::ZERO, U256::from(300u64), U256::from(30u64)).unwrap();
    let (_, _, _, amount2, _, freq2, _, _, _) = c.get_beneficiary(alice, U256::ZERO).unwrap();
    assert_eq!(amount2, U256::from(300u64));
    assert_eq!(freq2, U256::from(30u64));

    // get_pending_auto_payments -> none because no balance
    let pending = c.get_pending_auto_payments(alice);
    assert!(pending.is_empty());

    // Seed balance so pending appears
let contract_addr = c.vm().contract_address();
    seed_token_balance_and_approve(token, alice, contract_addr, U256::from(1_000u64));
    vm.set_sender(alice);
    c.deposit_balance(token, U256::from(1_000u64)).unwrap();

    // Now pending should contain index 0 (because last_payment==0 and user has balance >= amount)
    let pending2 = c.get_pending_auto_payments(alice);
    assert_eq!(pending2.len(), 1);
    assert_eq!(pending2[0], U256::from(0u64));

    // estimate next payment time -> since last_payment == 0 should return current block timestamp (can execute now)
    let est = c.estimate_next_payment_time(alice, U256::ZERO).unwrap();
    assert!(est > U256::ZERO || est == U256::from(vm.block_timestamp())); // implementation returns block_timestamp if last_payment==0

    // Remove beneficiary and ensure subsequent get fails
    vm.set_sender(alice);
    c.remove_beneficiary(U256::ZERO).unwrap();
    let err = c.get_beneficiary(alice, U256::ZERO).unwrap_err();
    match err {
        RemittanceErrors::BeneficiaryNotFound(_) => {}
        _ => panic!("expected BeneficiaryNotFound"),
    }
}

#[test]
fn execute_auto_payment_and_frequency_lock() {
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    let owner = address!("0x1000000000000000000000000000000000000001");
    let treasury = address!("0x2000000000000000000000000000000000000002");
    vm.set_sender(owner);
    c.constructor(treasury).unwrap();

    let alice = address!("0xA11CE00000000000000000000000000000000000");
    let bob = address!("0xB0B0000000000000000000000000000000000000");
    vm.set_sender(alice);
    c.register_user("Alice".into(), "NG".into(), "0803".into()).unwrap();
    vm.set_sender(bob);
    c.register_user("Bob".into(), "GH".into(), "000".into()).unwrap();

    let token = put_token(MockERC20::deployed_at(address!("0xDDD0000000000000000000000000000000000000")));
    vm.set_sender(owner);
    c.add_supported_token(token).unwrap();

    // add beneficiary with daily frequency (1)
    vm.set_sender(alice);
    c.add_beneficiary(bob, "Bob".into(), "friend".into(), U256::from(50u64), token, U256::from(1u64)).unwrap();

    // seed alice internal balance by deposit path
    let contract_addr = c.vm().contract_address();
    seed_token_balance_and_approve(token, alice, contract_addr, U256::from(1_000u64));
    vm.set_sender(alice);
    c.deposit_balance(token, U256::from(1_000u64)).unwrap();

    // set block timestamp to known value
    vm.set_block_timestamp(1000);
    // execute
    c.execute_auto_payments(alice, U256::ZERO).unwrap();

    // beneficiary last_payment updated to 1000 and total_sent increased
    let (_, _, _, _, _, _, last_payment, _, total_sent) = c.get_beneficiary(alice, U256::ZERO).unwrap();
    assert_eq!(last_payment, U256::from(1000u64));
    assert_eq!(total_sent, U256::from(50u64));

    // trying to execute again immediately should fail with FrequencyNotMet
    let err = c.execute_auto_payments(alice, U256::ZERO).unwrap_err();
    match err {
        RemittanceErrors::FrequencyNotMet(_) => {}
        _ => panic!("expected FrequencyNotMet"),
    }

    // advance time by 2 days (2*86400)
    vm.set_block_timestamp(1000 + 2 * 86400);
    // Now should succeed again
    c.execute_auto_payments(alice, U256::ZERO).unwrap();
}

#[test]
fn batch_execute_auto_payments_returns_results() {
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    let owner = address!("0x1000000000000000000000000000000000000001");
    let treasury = address!("0x2000000000000000000000000000000000000002");
    vm.set_sender(owner);
    c.constructor(treasury).unwrap();

    // create two users and their beneficiaries
    let alice = address!("0xA11CE00000000000000000000000000000000000");
    let bob = address!("0xB0B0000000000000000000000000000000000000");
    let charlie = address!("0xC0C0000000000000000000000000000000000000");

    vm.set_sender(alice);
    c.register_user("Alice".into(), "NG".into(), "0803".into()).unwrap();
    vm.set_sender(bob);
    c.register_user("Bob".into(), "GH".into(), "000".into()).unwrap();
    vm.set_sender(charlie);
    c.register_user("Charlie".into(), "KE".into(), "000".into()).unwrap();

    let token = put_token(MockERC20::deployed_at(address!("0xEEE0000000000000000000000000000000000000")));
    vm.set_sender(owner);
    c.add_supported_token(token).unwrap();

    // beneficiaries
    vm.set_sender(alice);
    c.add_beneficiary(bob, "Bob".into(), "friend".into(), U256::from(10u64), token, U256::from(1u64)).unwrap();
    vm.set_sender(bob);
    c.add_beneficiary(charlie, "Charlie".into(), "friend".into(), U256::from(20u64), token, U256::from(1u64)).unwrap();

    // seed balances
    let contract_addr = c.vm().contract_address();
    seed_token_balance_and_approve(token, alice, contract_addr, U256::from(100u64));
    seed_token_balance_and_approve(token, bob, contract_addr, U256::from(100u64));

    vm.set_sender(alice);
    c.deposit_balance(token, U256::from(100u64)).unwrap();
    vm.set_sender(bob);
    c.deposit_balance(token, U256::from(100u64)).unwrap();

    // batch execute two entries: (alice, 0), (bob, 0)
    vm.set_sender(owner); // caller of batch execution can be owner or anyone; implementation only when_not_paused
    let res = c.batch_execute_auto_payments(vec![(alice, U256::ZERO), (bob, U256::ZERO)]).unwrap();
    // Both should succeed (true/true)
    assert_eq!(res.len(), 2);
    assert!(res[0]);
    assert!(res[1]);
}

#[test]
fn admin_only_and_pause_emergency_withdraw() {
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    let owner = address!("0x1000000000000000000000000000000000000001");
    let treasury = address!("0x2000000000000000000000000000000000000002");
    vm.set_sender(owner);
    c.constructor(treasury).unwrap();

    let not_owner = address!("0xDEAD000000000000000000000000000000000000");
    vm.set_sender(not_owner);
    let err = c.pause().unwrap_err();
    match err {
        RemittanceErrors::Unauthorized(_) => {}
        _ => panic!("expected Unauthorized"),
    }

    // owner can pause/unpause
    vm.set_sender(owner);
    c.pause().unwrap();
    let (_, _, _, paused, _) = c.get_contract_stats();
    assert!(paused);
    c.unpause().unwrap();
    let (_, _, _, paused2, _) = c.get_contract_stats();
    assert!(!paused2);

    // update platform fee valid & invalid
    vm.set_sender(owner);
    c.update_platform_fee(U256::from(80u64)).unwrap(); // ok
    let err = c.update_platform_fee(U256::from(200u64)).unwrap_err(); // > 100 bps not allowed
    match err {
        RemittanceErrors::InvalidConfiguration(_) => {}
        _ => panic!("expected InvalidConfiguration"),
    }

    // emergency withdraw: put some tokens in contract, ensure owner can withdraw
    let token = put_token(MockERC20::deployed_at(address!("0xFFF0000000000000000000000000000000000000")));
    // seed contract token balance directly
    TOKENS.with(|m| {
        let mut map = m.borrow_mut();
        let t = map.get_mut(&token).unwrap();
        t.mint(c.vm().contract_address(), U256::from(1_000u64));
    });

    // not owner cannot emergency_withdraw
    vm.set_sender(not_owner);
    let err = c.emergency_withdraw(token, U256::from(100u64)).unwrap_err();
    match err {
        RemittanceErrors::Unauthorized(_) => {}
        _ => panic!("expected Unauthorized"),
    }

    vm.set_sender(owner);
    c.emergency_withdraw(token, U256::from(100u64)).unwrap();

    // verify owner balance increased
    TOKENS.with(|m| {
        let map = m.borrow();
        let t = map.get(&token).unwrap();
        assert_eq!(t.balance_of(owner), U256::from(100u64));
    });
}

#[test]
fn pause_blocks_mutations() {
    let vm = TestVM::default();
    let mut c = UniversalRemittance::from(&vm);

    let owner = address!("0x1000000000000000000000000000000000000001");
    vm.set_sender(owner);
    c.constructor(address!("0x2000000000000000000000000000000000000002")).unwrap();

    let alice = address!("0xA11CE00000000000000000000000000000000000");
    vm.set_sender(alice);
    // when unpaused - register works
    c.register_user("Alice".into(), "NG".into(), "0803".into()).unwrap();

    // pause
    vm.set_sender(owner);
    c.pause().unwrap();

    // now register attempt by another user should fail with ContractPaused
    let other = address!("0x1111000000000000000000000000000000000000");
    vm.set_sender(other);
    let err = c.register_user("Joe".into(), "NG".into(), "000".into()).unwrap_err();
    match err {
        RemittanceErrors::ContractPaused(_) => {}
        _ => panic!("expected ContractPaused"),
    }

    // unpause and attempt register again
    vm.set_sender(owner);
    c.unpause().unwrap();
    vm.set_sender(other);
    c.register_user("Joe".into(), "NG".into(), "000".into()).unwrap();
}
