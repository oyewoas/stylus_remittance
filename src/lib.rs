// Allow `cargo stylus export-abi` to generate a main function.
#![cfg_attr(not(any(test, feature = "export-abi")), no_main)]
#![cfg_attr(not(any(test, feature = "export-abi")), no_std)]

#[macro_use]
extern crate alloc;

use alloc::{string::String, vec::Vec};

use stylus_sdk::{
    alloy_primitives::{address, Address, U256}, alloy_sol_types::sol, console, prelude::*, storage::StorageType
};

// Error and event definitions
sol! {
    #[derive(Debug)]
    error Unauthorized();
    #[derive(Debug)]
    error InvalidConfiguration();
    #[derive(Debug)]
    error InsufficientBalance();
    #[derive(Debug)]
    error TransferFailed();
    #[derive(Debug)]
    error InvalidRecipients();
    #[derive(Debug)]
    error ExceedsLimit();
    #[derive(Debug)]
    error ContractPaused();
    #[derive(Debug)]
    error InvalidAmount();
    #[derive(Debug)]
    error NotRegistered();
    #[derive(Debug)]
    error FrequencyNotMet();
    #[derive(Debug)]
    error UserAlreadyRegistered();
    #[derive(Debug)]
    error BeneficiaryNotFound();
    #[derive(Debug)]
    error InvalidFrequency();
    #[derive(Debug)]
    error NotSupportedToken();

    event UserRegistered(address indexed user, string name, string country);
    event PaymentSent(address indexed sender, address indexed recipient, uint256 amount, address token, uint256 paymentType);
    event BeneficiaryAdded(address indexed user, address indexed beneficiary, string name, uint256 amount, address token, uint256 frequency);
    event BeneficiaryUpdated(address indexed user, address indexed beneficiary, uint256 amount, uint256 frequency);
    event BeneficiaryRemoved(address indexed user, address indexed beneficiary);
    event AutoPaymentExecuted(address indexed sender, address indexed beneficiary, uint256 amount, address token, uint256 executionId);
    event BalanceDeposited(address indexed user, address token, uint256 amount);
    event BalanceWithdrawn(address indexed user, address token, uint256 amount);
}

#[derive(SolidityError, Debug)]
pub enum RemittanceErrors {
    Unauthorized(Unauthorized),
    InvalidConfiguration(InvalidConfiguration),
    UserAlreadyRegistered(UserAlreadyRegistered),
    InsufficientBalance(InsufficientBalance),
    TransferFailed(TransferFailed),
    InvalidRecipients(InvalidRecipients),
    ExceedsLimit(ExceedsLimit),
    ContractPaused(ContractPaused),
    InvalidAmount(InvalidAmount),
    NotSupportedToken(NotSupportedToken),
    FrequencyNotMet(FrequencyNotMet),
    NotRegistered(NotRegistered),
    BeneficiaryNotFound(BeneficiaryNotFound),
    InvalidFrequency(InvalidFrequency),
}

// ERC20 interface
sol_interface! {
    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
    }
}

// Storage structures
sol_storage! {
    pub struct UserProfile {
        string name;
        string country;
        string phone_number;
        bool is_active;
        uint256 total_sent;
        uint256 total_received;
        uint256 registration_time;
        mapping(address => uint256) token_balances; // Internal balances for auto-payments
    }
    
    pub struct Beneficiary {
        address beneficiary_address;
        string name;
        string relationship; // "family", "friend", "business", etc.
        uint256 amount;
        address token;
        uint256 frequency; // 0=manual, 1=daily, 7=weekly, 30=monthly, 365=yearly
        uint256 last_payment;
        bool is_active;
        uint256 total_sent;
    }
    
    pub struct Payment {
        address sender;
        address recipient;
        uint256 amount;
        address token;
        uint256 timestamp;
        uint256 payment_type; // 0=manual, 1=auto, 2=scheduled
        string note;
        bool completed;
    }

    #[entrypoint]
    pub struct UniversalRemittance {
        address owner;
        bool paused;
        address treasury;
        uint256 platform_fee_percent; // In basis points (50 = 0.5%)
        uint256 payment_count;
        uint256 execution_count;
        
        // User management
        mapping(address => UserProfile) users;
        mapping(address => bool) registered_users;
        
        // Beneficiary management  
        mapping(address => mapping(uint256 => Beneficiary)) user_beneficiaries; // user => index => beneficiary
        mapping(address => uint256) beneficiary_counts; // user => count
        
        // Payment tracking
        mapping(uint256 => Payment) payments; // payment ID => payment
        
        // Supported tokens
        mapping(address => bool) supported_tokens;
        
        // Daily limits (optional, can be 0 for unlimited)
        mapping(address => uint256) daily_limits;
        mapping(address => mapping(uint256 => uint256)) daily_spent; // user => day => amount
    }
}

// Main contract implementation
#[public]
impl UniversalRemittance {
    
    #[constructor]
    pub fn constructor(&mut self, treasury: Address) -> Result<(), RemittanceErrors> {
        if self.owner.get() != Address::ZERO {
            return Err(RemittanceErrors::Unauthorized(Unauthorized {}));
        }
        
        self.owner.set(self.vm().tx_origin());
        self.treasury.set(treasury);
        self.platform_fee_percent.set(U256::from(50)); // 0.5%
        
        // Add common stablecoins
        let usdc_arbitrum = address!("af88d065e77c8cC2239327C5EDb3A432268e5831");
        let usdt_arbitrum = address!("Fd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9");
        self.supported_tokens.setter(usdc_arbitrum).set(true);
        self.supported_tokens.setter(usdt_arbitrum).set(true);
        
        Ok(())
    }

    // === USER MANAGEMENT === //
    
    pub fn register_user(
        &mut self,
        name: String,
        country: String,
        phone_number: String,
    ) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;
        let sender = self.vm().msg_sender();
        
        if self.registered_users.get(sender) {
            return Err(RemittanceErrors::UserAlreadyRegistered(UserAlreadyRegistered {}));
        }
        
        let block_timestamp = U256::from(self.vm().block_timestamp());
        
        let mut profile = self.users.setter(sender);
        profile.name.set_str(&name);
        profile.country.set_str(&country);
        profile.phone_number.set_str(&phone_number);
        profile.is_active.set(true);
        profile.total_sent.set(U256::ZERO);
        profile.total_received.set(U256::ZERO);
        profile.registration_time.set(block_timestamp);
        
        self.registered_users.setter(sender).set(true);
        
        log(self.vm(), UserRegistered {
            user: sender,
            name,
            country,
        });
        
        Ok(())
    }

    // === BALANCE MANAGEMENT === //
    
    pub fn deposit_balance(&mut self, token: Address, amount: U256) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;
        self.only_registered()?;
        if amount == U256::ZERO {
            return Err(RemittanceErrors::InvalidAmount(InvalidAmount {}));
        }
        if !self.supported_tokens.get(token) {
            return Err(RemittanceErrors::NotSupportedToken(NotSupportedToken {}));
        }
        
        let sender = self.vm().msg_sender();
        let contract_addr = self.vm().contract_address();
        let token_contract = IERC20::new(token);

        // Transfer tokens to contract
        match token_contract.transfer_from(&mut *self, sender, contract_addr, amount) {
            Ok(success) => {
                if !success {
                    return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                }
            }
            Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
        }
        
        // Update internal balance
        let mut user_profile = self.users.setter(sender);
        let current_balance = user_profile.token_balances.get(token);
        user_profile.token_balances.setter(token).set(current_balance + amount);
        
        log(self.vm(), BalanceDeposited {
            user: sender,
            token,
            amount,
        });
        
        Ok(())
    }
    
    pub fn withdraw_balance(&mut self, token: Address, amount: U256) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;
        self.only_registered()?;
        
        if !self.supported_tokens.get(token) || amount == U256::ZERO {
            return Err(RemittanceErrors::InvalidConfiguration(InvalidConfiguration {}));
        }
        
        let sender = self.vm().msg_sender();
        let mut user_profile = self.users.setter(sender);
        let current_balance = user_profile.token_balances.get(token);
        
        if current_balance < amount {
            return Err(RemittanceErrors::InsufficientBalance(InsufficientBalance {}));
        }
        
        // Update internal balance
        user_profile.token_balances.setter(token).set(current_balance - amount);
        
        // Transfer tokens to user
        let token_contract = IERC20::new(token);
        match token_contract.transfer(&mut *self, sender, amount) {
            Ok(success) => {
                if !success {
                    return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                }
            }
            Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
        }
        
        log(self.vm(), BalanceWithdrawn {
            user: sender,
            token,
            amount,
        });
        
        Ok(())
    }

    // === PAYMENT FUNCTIONS === //
    
    pub fn send_payment(
        &mut self,
        recipient: Address,
        amount: U256,
        token: Address,
        note: String,
    ) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;
        self.only_registered()?;
        
        if !self.supported_tokens.get(token) || amount == U256::ZERO {
            return Err(RemittanceErrors::InvalidConfiguration(InvalidConfiguration {}));
        }
        
        let sender = self.vm().msg_sender();
        
        // Check daily limit if set
        if !self.check_daily_limit(sender, amount) {
            return Err(RemittanceErrors::ExceedsLimit(ExceedsLimit {}));
        }
        
        let token_contract = IERC20::new(token);
        let contract_addr = self.vm().contract_address();
        
        // Transfer tokens to contract
        match token_contract.transfer_from(&mut *self, sender, contract_addr, amount) {
            Ok(success) => {
                if !success {
                    return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                }
            }
            Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
        }
        
        // Calculate fee
        let platform_fee = (amount * self.platform_fee_percent.get()) / U256::from(10000);
        let net_amount = amount.checked_sub(platform_fee)
            .ok_or(RemittanceErrors::InvalidAmount(InvalidAmount {}))?;
        
        // Send to recipient
        match token_contract.transfer(&mut *self, recipient, net_amount) {
            Ok(success) => {
                if !success {
                    return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                }
            }
            Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
        }
        
        // Send fee to treasury
        if platform_fee > U256::ZERO {
            let treasury_addr = self.treasury.get();
            match token_contract.transfer(&mut *self, treasury_addr, platform_fee) {
                Ok(success) => {
                    if !success {
                        return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                    }
                }
                Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
            }
        }
        
        // Record payment
        let payment_id = self.payment_count.get();
        let block_timestamp = U256::from(self.vm().block_timestamp());
        
        let mut payment = self.payments.setter(payment_id);
        payment.sender.set(sender);
        payment.recipient.set(recipient);
        payment.amount.set(amount);
        payment.token.set(token);
        payment.timestamp.set(block_timestamp);
        payment.payment_type.set(U256::ZERO); // Manual payment
        payment.note.set_str(&note);
        payment.completed.set(true);
        
        self.payment_count.set(payment_id + U256::from(1));
        
        // Update user stats
        let mut sender_profile = self.users.setter(sender);
        let sender_total = sender_profile.total_sent.get();
        sender_profile.total_sent.set(sender_total + amount);
        
        if self.registered_users.get(recipient) {
            let mut recipient_profile = self.users.setter(recipient);
            let recipient_total = recipient_profile.total_received.get();
            recipient_profile.total_received.set(recipient_total + net_amount);
        }
        
        // Update daily spent
        self.update_daily_spent(sender, amount);
        
        log(self.vm(), PaymentSent {
            sender,
            recipient,
            amount,
            token,
            paymentType: U256::ZERO,
        });
        
        Ok(())
    }

    // === BENEFICIARY MANAGEMENT === //
    
    pub fn add_beneficiary(
        &mut self,
        beneficiary_address: Address,
        name: String,
        relationship: String,
        amount: U256,
        token: Address,
        frequency: U256, // 0=manual, 1=daily, 7=weekly, 30=monthly, 365=yearly
    ) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;
        self.only_registered()?;
        
        if !self.supported_tokens.get(token) || amount == U256::ZERO {
            return Err(RemittanceErrors::InvalidConfiguration(InvalidConfiguration {}));
        }
        
        // Validate frequency
        if frequency != U256::ZERO && frequency != U256::from(1) && frequency != U256::from(7) && 
           frequency != U256::from(30) && frequency != U256::from(365) {
            return Err(RemittanceErrors::InvalidFrequency(InvalidFrequency {}));
        }
        
        let sender = self.vm().msg_sender();
        let beneficiary_count = self.beneficiary_counts.get(sender);
        
        let mut user_beneficiaries_setter = self.user_beneficiaries.setter(sender);
        let mut beneficiary = user_beneficiaries_setter.setter(beneficiary_count);
        beneficiary.beneficiary_address.set(beneficiary_address);
        beneficiary.name.set_str(&name);
        beneficiary.relationship.set_str(&relationship);
        beneficiary.amount.set(amount);
        beneficiary.token.set(token);
        beneficiary.frequency.set(frequency);
        beneficiary.last_payment.set(U256::ZERO);
        beneficiary.is_active.set(true);
        beneficiary.total_sent.set(U256::ZERO);
        
        self.beneficiary_counts.setter(sender).set(beneficiary_count + U256::from(1));
        
        log(self.vm(), BeneficiaryAdded {
            user: sender,
            beneficiary: beneficiary_address,
            name,
            amount,
            token,
            frequency,
        });
        
        Ok(())
    }
    
    pub fn update_beneficiary(
        &mut self,
        beneficiary_index: U256,
        amount: U256,
        frequency: U256,
    ) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;
        self.only_registered()?;
        
        let sender = self.vm().msg_sender();
        let beneficiary_count = self.beneficiary_counts.get(sender);
        
        if beneficiary_index >= beneficiary_count {
            return Err(RemittanceErrors::BeneficiaryNotFound(BeneficiaryNotFound {}));
        }
        
        // Validate frequency
        if frequency != U256::ZERO && frequency != U256::from(1) && frequency != U256::from(7) && 
           frequency != U256::from(30) && frequency != U256::from(365) {
            return Err(RemittanceErrors::InvalidFrequency(InvalidFrequency {}));
        }
        
        let mut user_beneficiaries_setter = self.user_beneficiaries.setter(sender);
        let mut beneficiary = user_beneficiaries_setter.setter(beneficiary_index);
        let beneficiary_address = beneficiary.beneficiary_address.get();
        
        if !beneficiary.is_active.get() {
            return Err(RemittanceErrors::BeneficiaryNotFound(BeneficiaryNotFound {}));
        }
        
        beneficiary.amount.set(amount);
        beneficiary.frequency.set(frequency);
        
        log(self.vm(), BeneficiaryUpdated {
            user: sender,
            beneficiary: beneficiary_address,
            amount,
            frequency,
        });
        
        Ok(())
    }
    
    pub fn remove_beneficiary(&mut self, beneficiary_index: U256) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;
        self.only_registered()?;
        
        let sender = self.vm().msg_sender();
        let beneficiary_count = self.beneficiary_counts.get(sender);
        
        if beneficiary_index >= beneficiary_count {
            return Err(RemittanceErrors::BeneficiaryNotFound(BeneficiaryNotFound {}));
        }
        
        let mut user_beneficiaries_setter = self.user_beneficiaries.setter(sender);
        let mut beneficiary = user_beneficiaries_setter.setter(beneficiary_index);
        let beneficiary_address = beneficiary.beneficiary_address.get();
        
        if !beneficiary.is_active.get() {
            return Err(RemittanceErrors::BeneficiaryNotFound(BeneficiaryNotFound {}));
        }
        
        beneficiary.is_active.set(false);
        
        log(self.vm(), BeneficiaryRemoved {
            user: sender,
            beneficiary: beneficiary_address,
        });
        
        Ok(())
    }

    // === AUTO PAYMENT EXECUTION === //
    
    pub fn execute_auto_payments(&mut self, user: Address, beneficiary_index: U256) -> Result<(), RemittanceErrors> {
        self.when_not_paused()?;

        // Get block timestamp before any mutable borrow
        let current_time = U256::from(self.vm().block_timestamp());

        let beneficiary_count = self.beneficiary_counts.get(user);
        if beneficiary_index >= beneficiary_count {
            return Err(RemittanceErrors::BeneficiaryNotFound(BeneficiaryNotFound {}));
        }

        let mut user_beneficiaries_setter = self.user_beneficiaries.setter(user);
        let beneficiary = user_beneficiaries_setter.setter(beneficiary_index);
        if !beneficiary.is_active.get() || beneficiary.frequency.get() == U256::ZERO {
            return Err(RemittanceErrors::InvalidConfiguration(InvalidConfiguration {}));
        }

        let last_payment = beneficiary.last_payment.get();
        let frequency_seconds = beneficiary.frequency.get() * U256::from(86400); // Convert days to seconds

        if last_payment > U256::ZERO && (current_time - last_payment) < frequency_seconds {
            return Err(RemittanceErrors::FrequencyNotMet(FrequencyNotMet {}));
        }

        let amount = beneficiary.amount.get();
        let token = beneficiary.token.get();
        let beneficiary_address = beneficiary.beneficiary_address.get();

        // Check user's internal balance
        let user_profile = self.users.get(user);
        let user_balance = user_profile.token_balances.get(token);

        if user_balance < amount {
            return Err(RemittanceErrors::InsufficientBalance(InsufficientBalance {}));
        }

        // Calculate fee
        let platform_fee = (amount * self.platform_fee_percent.get()) / U256::from(10000);
        let net_amount = amount.checked_sub(platform_fee)
            .ok_or(RemittanceErrors::InvalidAmount(InvalidAmount {}))?;

        // Update user's internal balance
        {
            let mut user_profile_setter = self.users.setter(user);
            user_profile_setter.token_balances.setter(token).set(user_balance - amount);
        }

        // Transfer to beneficiary
        let token_contract = IERC20::new(token);
        let transfer_result = token_contract.transfer(&mut *self, beneficiary_address, net_amount);
        match transfer_result {
            Ok(success) => {
                if !success {
                    return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                }
            }
            Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
        }

        // Send fee to treasury
        if platform_fee > U256::ZERO {
            let treasury_addr = self.treasury.get();
            let fee_result = token_contract.transfer(&mut *self, treasury_addr, platform_fee);
            match fee_result {
                Ok(success) => {
                    if !success {
                        return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                    }
                }
                Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
            }
        }

        // Re-borrow to update beneficiary
        {
            let mut user_beneficiaries_setter = self.user_beneficiaries.setter(user);
            let mut beneficiary = user_beneficiaries_setter.setter(beneficiary_index);
            beneficiary.last_payment.set(current_time);
            let beneficiary_total = beneficiary.total_sent.get();
            beneficiary.total_sent.set(beneficiary_total + amount);
        }

        // Update user stats
        {
            let mut user_profile_setter = self.users.setter(user);
            let user_total = user_profile_setter.total_sent.get();
            user_profile_setter.total_sent.set(user_total + amount);
        }

        // Update recipient stats if registered
        if self.registered_users.get(beneficiary_address) {
            let mut recipient_profile = self.users.setter(beneficiary_address);
            let recipient_total = recipient_profile.total_received.get();
            recipient_profile.total_received.set(recipient_total + net_amount);
        }

        // Record execution
        let execution_id = self.execution_count.get();
        self.execution_count.set(execution_id + U256::from(1));

        log(self.vm(), AutoPaymentExecuted {
            sender: user,
            beneficiary: beneficiary_address,
            amount,
            token,
            executionId: execution_id,
        });

        Ok(())
    }

    // === ADMIN FUNCTIONS === //
    
    pub fn add_supported_token(&mut self, token: Address) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        self.supported_tokens.setter(token).set(true);
        Ok(())
    }
    
    pub fn remove_supported_token(&mut self, token: Address) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        self.supported_tokens.setter(token).set(false);
        Ok(())
    }
    
    pub fn set_daily_limit(&mut self, user: Address, limit: U256) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        self.daily_limits.setter(user).set(limit);
        Ok(())
    }
    
    pub fn pause(&mut self) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        self.paused.set(true);
        Ok(())
    }
    
    pub fn unpause(&mut self) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        self.paused.set(false);
        Ok(())
    }

    // === VIEW FUNCTIONS === //
    
    pub fn get_user_profile(&self, user: Address) -> (String, String, String, bool, U256, U256, U256) {
        let profile = self.users.get(user);
        (
            profile.name.get_string(),
            profile.country.get_string(),
            profile.phone_number.get_string(),
            profile.is_active.get(),
            profile.total_sent.get(),
            profile.total_received.get(),
            profile.registration_time.get(),
        )
    }
    
    pub fn get_user_balance(&self, user: Address, token: Address) -> U256 {
        self.users.get(user).token_balances.get(token)
    }
    
    pub fn get_beneficiary(&self, user: Address, index: U256) -> Result<(Address, String, String, U256, Address, U256, U256, bool, U256), RemittanceErrors> {
        let beneficiary_count = self.beneficiary_counts.get(user);
        if index >= beneficiary_count {
            return Err(RemittanceErrors::BeneficiaryNotFound(BeneficiaryNotFound {}));
        }
        
        let user_beneficiaries = self.user_beneficiaries.get(user);
        let beneficiary = user_beneficiaries.get(index);
        Ok((
            beneficiary.beneficiary_address.get(),
            beneficiary.name.get_string(),
            beneficiary.relationship.get_string(),
            beneficiary.amount.get(),
            beneficiary.token.get(),
            beneficiary.frequency.get(),
            beneficiary.last_payment.get(),
            beneficiary.is_active.get(),
            beneficiary.total_sent.get(),
        ))
    }
    
    pub fn get_beneficiary_count(&self, user: Address) -> U256 {
        self.beneficiary_counts.get(user)
    }
    
    pub fn get_payment(&self, payment_id: U256) -> Result<(Address, Address, U256, Address, U256, U256, String, bool), RemittanceErrors> {
        if payment_id >= self.payment_count.get() {
            return Err(RemittanceErrors::InvalidConfiguration(InvalidConfiguration {}));
        }
        
        let payment = self.payments.get(payment_id);
        Ok((
            payment.sender.get(),
            payment.recipient.get(),
            payment.amount.get(),
            payment.token.get(),
            payment.timestamp.get(),
            payment.payment_type.get(),
            payment.note.get_string(),
            payment.completed.get(),
        ))
    }
    
    pub fn is_token_supported(&self, token: Address) -> bool {
        self.supported_tokens.get(token)
    }
    
    pub fn get_daily_limit(&self, user: Address) -> U256 {
        self.daily_limits.get(user)
    }
    
    pub fn get_daily_spent(&self, user: Address) -> U256 {
        let today = U256::from(self.vm().block_timestamp() / 86400);
        self.daily_spent.getter(user).get(today)
    }
    
    pub fn get_contract_stats(&self) -> (U256, U256, U256, bool, Address) {
        (
            self.payment_count.get(),
            self.execution_count.get(),
            self.platform_fee_percent.get(),
            self.paused.get(),
            self.treasury.get(),
        )
    }

    // === INTERNAL FUNCTIONS === //
    
    fn only_owner(&self) -> Result<(), RemittanceErrors> {
        if self.vm().msg_sender() != self.owner.get() {
            return Err(RemittanceErrors::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }
    
    fn only_registered(&self) -> Result<(), RemittanceErrors> {
        if !self.registered_users.get(self.vm().msg_sender()) {
            return Err(RemittanceErrors::NotRegistered(NotRegistered {}));
        }
        Ok(())
    }
    
    fn when_not_paused(&self) -> Result<(), RemittanceErrors> {
        if self.paused.get() {
            return Err(RemittanceErrors::ContractPaused(ContractPaused {}));
        }
        Ok(())
    }
    
    fn check_daily_limit(&self, user: Address, amount: U256) -> bool {
        let daily_limit = self.daily_limits.get(user);
        if daily_limit == U256::ZERO {
            return true; // No limit set
        }
        
        let today = U256::from(self.vm().block_timestamp() / 86400);
        let today_spent = self.daily_spent.getter(user).get(today);
        today_spent + amount <= daily_limit
    }
    
    fn update_daily_spent(&mut self, user: Address, amount: U256) {
        let today = U256::from(self.vm().block_timestamp() / 86400);
        let current_spent = self.daily_spent.getter(user).get(today);
        self.daily_spent.setter(user).setter(today).set(current_spent + amount);
    }
    
    // === BATCH OPERATIONS === //
    
    pub fn batch_execute_auto_payments(&mut self, users_and_indices: Vec<(Address, U256)>) -> Result<Vec<bool>, RemittanceErrors> {
        self.when_not_paused()?;
        
        let mut results = Vec::new();
        
        for (user, beneficiary_index) in users_and_indices {
            match self.execute_auto_payments(user, beneficiary_index) {
                Ok(()) => results.push(true),
                Err(_) => results.push(false),
            }
        }
        
        Ok(results)
    }
    
    // === UTILITY FUNCTIONS === //
    
    pub fn get_pending_auto_payments(&self, user: Address) -> Vec<U256> {
        let mut pending = Vec::new();
        let beneficiary_count = self.beneficiary_counts.get(user);
        let current_time = U256::from(self.vm().block_timestamp());
        
        for i in 0..beneficiary_count.as_limbs()[0] as usize {
            let index = U256::from(i);
            let user_beneficiaries = self.user_beneficiaries.get(user);
            let beneficiary = user_beneficiaries.get(index);
            
            if !beneficiary.is_active.get() || beneficiary.frequency.get() == U256::ZERO {
                continue;
            }
            
            let last_payment = beneficiary.last_payment.get();
            let frequency_seconds = beneficiary.frequency.get() * U256::from(86400);
            
            if last_payment == U256::ZERO || (current_time - last_payment) >= frequency_seconds {
                // Check if user has sufficient balance
                let amount = beneficiary.amount.get();
                let token = beneficiary.token.get();
                let user_balance = self.users.get(user).token_balances.get(token);
                
                if user_balance >= amount {
                    pending.push(index);
                }
            }
        }
        
        pending
    }
    
    pub fn estimate_next_payment_time(&self, user: Address, beneficiary_index: U256) -> Result<U256, RemittanceErrors> {
        let beneficiary_count = self.beneficiary_counts.get(user);
        if beneficiary_index >= beneficiary_count {
            return Err(RemittanceErrors::BeneficiaryNotFound(BeneficiaryNotFound {}));
        }
        
        let user_beneficiaries = self.user_beneficiaries.get(user);
        let beneficiary = user_beneficiaries.get(beneficiary_index);
        if !beneficiary.is_active.get() || beneficiary.frequency.get() == U256::ZERO {
            return Ok(U256::ZERO); // Manual payment, no scheduled time
        }
        
        let last_payment = beneficiary.last_payment.get();
        let frequency_seconds = beneficiary.frequency.get() * U256::from(86400);
        
        if last_payment == U256::ZERO {
            return Ok(U256::from(self.vm().block_timestamp())); // Can be executed now
        }
        
        Ok(last_payment + frequency_seconds)
    }
    
    // === EMERGENCY FUNCTIONS === //
    
    pub fn emergency_withdraw(&mut self, token: Address, amount: U256) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        
        let token_contract = IERC20::new(token);
        let owner_addr = self.owner.get();
        
        match token_contract.transfer(&mut *self, owner_addr, amount) {
            Ok(success) => {
                if !success {
                    return Err(RemittanceErrors::TransferFailed(TransferFailed {}));
                }
            }
            Err(_) => return Err(RemittanceErrors::TransferFailed(TransferFailed {})),
        }
        
        Ok(())
    }
    
    pub fn update_platform_fee(&mut self, new_fee_percent: U256) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        
        // Max fee of 1% (100 basis points)
        if new_fee_percent > U256::from(100) {
            return Err(RemittanceErrors::InvalidConfiguration(InvalidConfiguration {}));
        }
        
        self.platform_fee_percent.set(new_fee_percent);
        Ok(())
    }
    
    pub fn update_treasury(&mut self, new_treasury: Address) -> Result<(), RemittanceErrors> {
        self.only_owner()?;
        
        if new_treasury == Address::ZERO {
            return Err(RemittanceErrors::InvalidConfiguration(InvalidConfiguration {}));
        }
        
        self.treasury.set(new_treasury);
        Ok(())
    }
}