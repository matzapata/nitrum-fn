(module
  (memory (export "memory") 1)
  (func (export "invoke") (param $ptr i32) (param $len i32) (result i32)
    local.get $len
  )
)
