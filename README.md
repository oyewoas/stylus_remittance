# Crossborder Remittance Protocol

## Overview

**Crossborder Remittance Protocol** is a smart contract system for managing global remittances, built for the Ethereum ecosystem (Stylus/Arbitrum). It enables users to deposit stablecoins, register beneficiaries, and automate or manually send payments with built-in fee management, daily limits, and robust user controls.

### Features

- **User Registration:**  
  Users can register with name, country, and phone number. Each user has an internal balance for supported tokens.

- **Supported Tokens:**  
  Out-of-the-box support for USDC and USDT (Arbitrum addresses). Admins can add/remove supported ERC20 tokens.

- **Deposit & Withdraw:**  
  Users deposit supported tokens into their internal balance and can withdraw at any time.

- **Beneficiary Management:**  
  Add, update, or remove beneficiaries with custom names, relationships, payment amounts, tokens, and payment frequency (manual, daily, weekly, monthly, yearly).

- **Automated Payments:**  
  Schedule recurring payments to beneficiaries. The contract enforces frequency locks and checks user balances before execution.

- **Manual Payments:**  
  Send one-off payments to any address, with optional notes.

- **Fee Management:**  
  Platform fee (default 0.5%) is deducted from payments and sent to a treasury address. Admin can update fee and treasury.

- **Daily Limits:**  
  Admins can set daily spending limits for users.

- **Emergency Controls:**  
  Admin can pause/unpause the contract, perform emergency withdrawals, and manage supported tokens.

- **Batch Operations:**  
  Batch execution of auto-payments for multiple users.

- **Events & Tracking:**  
  Emits events for all major actions (registration, payments, beneficiary changes, deposits/withdrawals). Tracks payment history and user stats.

## Technologies Used

- **Rust**  
  Core contract logic and tests are written in Rust for Stylus.

- **Stylus SDK**  
  Used for Ethereum-compatible smart contract development on Arbitrum.

- **Alloy Primitives & Sol Types**  
  For Ethereum address, U256, and Solidity-like type handling.

- **ERC20 Interface**  
  Interacts with standard ERC20 tokens for deposits, withdrawals, and payments.

- **Arbitrum/Layer 2**  
  Designed for deployment on Arbitrum using Stylus.

- **Automated Testing**  
  Comprehensive unit tests with mocked ERC20 tokens and in-memory state.

## How It Works

1. **Deploy the contract** and set the treasury address.
2. **Users register** and deposit supported tokens.
3. **Add beneficiaries** with payment details and frequency.
4. **Send payments** manually or let the contract execute scheduled payments.
5. **Admin manages** supported tokens, fees, limits, and emergency controls.

## Getting Started

1. **Clone the repo:**  
   `git clone ...`
2. **Install Rust & Stylus toolchain**
3. **Run tests:**  
   `cargo test`
4. **Deploy to Arbitrum Stylus (see Stylus docs)**

## File Structure

- `src/lib.rs` — Main contract logic
- `tests/remittance_protocol.rs` — Unit tests and ERC20 mocks
- `README.md` — Project documentation

## License

MIT

---

**For more details, see the source code and tests.**