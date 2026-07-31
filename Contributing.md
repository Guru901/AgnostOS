# Contributing guidelines

Thanks a lot ! for consdiering to contribute to this project this project is focused on learning how a os works by implementing things that a os consists of.

please follow the following coding style when submitting code.

## Coding style

### 1. Document Every `unsafe` Block
We use `unsafe` only when absolutely necessary. Every single `unsafe` block must be accompanied by a comment explicitly explaining why the operation is guaranteed to be safe in that context.
* **Rule**: No `unsafe` block will be merged without a preceding `// SAFETY:` comment.
* **Example**:
  ```rust
  // SAFETY: The pointer is guaranteed to be aligned and points to a valid,
  // initialized static memory block that is never mutated concurrently.
  unsafe { *my_pointer }
  ```

### 2. Encapsulate Low-Level Pointers
Raw, low-level pointers must never be exposed to the public API. 
* Wrap all raw pointers inside private or internal structures.
* Expose only safe, validated methods publicly.
* Ensure your wrapper structures handle bound checks and lifetime enforcement internally.

### 3. Zero Tolerance for Kernel Panics & Triple Faults
A system crash is the worst possible outcome. Your code must be resilient and predictable.
* **Never use unwrap() or expect()**: Always propagate errors gracefully using `Result` or `Option` types.
* **Validate boundaries**: Check array indices and buffer capacities before accessing them.
* **Handle all failures**: Anticipate edge cases, hardware limits, and corrupted inputs. If a failure occurs, return an error to let the caller handle it.

### 4. Never Use Raw Integers (Strong Typing)
To prevent mixing up semantic values (like memory addresses, process IDs, or hardware ports), raw integers are banned in public signatures.
* Wrap raw primitive integers in domain-specific types or newtype structs.
* **Example**:
  ```rust
  // Bad
  fn configure_core(id: u32, speed: u64);

  // Good
  struct CoreId(u32);
  struct ClockSpeedHz(u64);
  fn configure_core(id: CoreId, speed: ClockSpeedHz);
  ```
