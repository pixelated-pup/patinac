# Introduction
Patina is a WIP semi-functional general-purpose programming language, comparable in target usage to languages such as Swift, C#, Go, and OCaml.

As well as all the expected features for a modern functional language (type inference, polymorphism, ADTs), Patina features:
- Mutable Value Semantics
- Typeclasses
- Opt-in Linear Types
- Algebraic Effects and Handlers

It compiles to native code using LLVM, and allows controlled usage of low-level pointers to implement fundamental data structures in-library, in a similar manner to Rust.

The current implementation is very limited, and is missing most features. 

# Examples
All of these are likely to change significantly. Each example will be labelled with whether it currently runs.

*Runs*
```def main(): () = {
    let mut a = 5
    let mut b = 6

    // Mutable/in-out parameters must be marked at the callsite
    swap(mut a, mut b)

    // Wouldn't compile, overlapping mutable places
    // swap(mut a, mut a)
}

def swap(mut a: Int, mut b: Int): () = { 
    let a_tmp = a
    a = b
    b = a_tmp
}
```