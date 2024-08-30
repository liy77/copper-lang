macro_rules! class_body {
    () => {
        
    };
}

/// Represents a class macro that generates a class with fields, methods, and companion objects.
/// Based on the Kotlin and C++ class syntax.
/// 
/// # Example
/// 
/// ```rust
/// use std::marker::PhantomData;
/// 
/// class!(self {
///    #[derive(Debug)]
///   class Foo {
///      private {
///         bar: i32
///     }
///  }
/// });
/// 
/// trait Bar {
///    fn get_kind(&self) -> i32;
/// }
/// 
/// class!(self {
///   #[derive(Debug)]
///  class PhantomClass<T>, extends Foo, implements Bar {
///     public {
///        phantom: PhantomData<T>
/// 
///       :self {
///         get_kind(): PhantomData<T> {
///           self.phantom
///        }
/// 
///       @const {
///         get_kind_const(): &PhantomData<T> {
///          &self.phantom
///       }
///    }
/// }
/// });
/// 
/// ```
#[macro_export]
macro_rules! class {
    ($sel:ident {
        $(#[$meta:meta])?
        $vis:vis class $name:ident$(<$($gen:ident),*>)? $(, extends $child:ty)? $(, implements $($impl:ty),*)? {
            $(public {
                $(#[$meta2:meta])*
                $($field:ident: $type:ty),*
    
                $(:self {
                    $(#[$method_meta:meta])?
                    $($method_name:ident($($arg_name:ident: $arg_type:ident),*): $method_return_type:ty $body:block)*
    
                    $(@const {
                        $(#[$const_method_meta:meta])?
                        $($method_name_const:ident($($arg_name_const:ident: $arg_type_const:ident),*): $method_return_type_const:ty $body_const:block)*
                    })?
                })?
    
                $(:companion {
                    $(#[$companion_method_meta:meta])?
                    $($method_name_companion:ident($($arg_name_companion:ident: $arg_type_companion:ident),*): $method_return_type_companion:ty $body_companion:block)*
                })?
            })?
    
            $(private {
                $(#[$meta3:meta])*
                $($field_private:ident: $type_private:ty),*
    
                $(:self {
                    $(#[$private_method_meta:meta])?
                    $($method_return_type_private:ident $method_name_private:ident($($arg_name_private:ident: $arg_type_private:ident),*) $body_private:block)*
    
                    $(@const {
                        $(#[$private_const_method_meta:meta])?
                        $($method_return_type_const_private:ident $method_name_const_private:ident($($arg_name_const_private:ident: $arg_type_const_private:ident),*) $body_const_private:block)*
                    })?
                })?
    
                $(:companion {
                    $(#[$private_companion_method_meta:meta])?
                    $($method_return_type_companion_private:ident $method_name_companion_private:ident($($arg_name_companion_private:ident: $arg_type_companion_private:ident),*) $body_companion_private:block)*
                })?
            })?
        }
    }
) => {
        $(#[$meta])?
        $vis struct $name$(<$($gen),*>)* {
            $(
                __child__: $child,
            )?

            $(
                $(#[$meta2])?
                $(pub $field: $type,)*
            )?

            $(
                $(#[$meta3])?
                $($field_private: $type_private),*
            )?
        }

        impl $(<$($gen),*>)* $name$(<$($gen),+>)* {
            $(
                $(
                    $(#[$method_meta])?
                    $(
                        pub fn $method_name(
                            &mut $sel,
                            $($arg_name: $arg_type),*
                        ) -> $method_return_type $body
                    )*
    
                    $(
                        $(#[$const_method_meta])?
                        $(
                            pub const fn $method_name_const(
                                &$sel,
                                $($arg_name_const: $arg_type_const),*
                            ) -> $method_return_type_const $body_const
                        )*
                    )?
                )*
    
                $(
                    $(#[$companion_method_meta])?
                    $(
                        pub fn $method_name_companion(
                            $($arg_name_companion: $arg_type_companion),*
                        ) -> $method_return_type_companion $body_companion
                    )*
                )?
            )?

            $(

                $(
                    $(#[$private_method_meta])?
                    $(
                        fn $method_name_private(
                            &mut $sel,
                            $($arg_name_private: $arg_type_private),*
                        ) -> $method_return_type_private $body_private
                    )*
    
                    $(
                        $(#[$private_const_method_meta])?
                        $(
                            const fn $method_name_const_private(
                                &$sel,
                                $($arg_name_const_private: $arg_type_const_private),*
                            ) -> $method_return_type_const_private $body_const_private
                        )*
                    )?
                )*
    
                $(
                    $(#[$private_companion_method_meta])?
                    $(
                        fn $method_name_companion_private(
                            $($arg_name_companion_private: $arg_type_companion_private),*
                        ) -> $method_return_type_companion_private $body_companion_private
                    )*
                )?
            )?
        }
    }
}


mod test {
    use std::marker::PhantomData;

    class!(self {
        #[derive(Debug)]
        class Foo {
            private {
                bar: i32
            }
        }
    });

    trait Bar {
        fn get_kind(&self) -> i32;
    }

    class!(self {
        #[derive(Debug)]
         class PhantomClass<T>, extends Foo, implements Bar {
            public {
                #[doc = "A phantom data field."]
                phantom: PhantomData<T>,
                value: T

                :self {
                    get_kind(): PhantomData<T> {
                        self.phantom
                    }

                    @const {
                        get_kind_const(): &PhantomData<T> {
                            &self.phantom
                        }
                    }
                }

                :companion {
                    new(kind: T): PhantomClass<T> {
                        PhantomClass {
                            __child__: Foo {
                                bar: 10
                            },
                            value: kind,
                            phantom: PhantomData
                        }
                    }
                }
            }
        }
    });

    #[test]
    fn test() {
        let mut phantom = PhantomClass::<i32>::new(10);

        println!("{:?}", phantom.get_kind());
        assert_eq!(phantom.get_kind_const(), &PhantomData);
    }
}