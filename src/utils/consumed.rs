pub enum Consumed {
    Consumed(isize),
    Empty,
}

pub trait ConsumedTrait {
    fn or<U: FnOnce() -> Consumed>(self, other: U) -> Consumed;
    fn consume(amount: isize) -> Consumed;
}

pub trait ConsumeVar<T> {
    fn consume_var(&self, var: &mut T) -> Consumed;
}

impl Consumed {
    pub fn is_consumed(&self) -> bool {
        match self {
            Consumed::Consumed(_) => true,
            Consumed::Empty => false,
        }
    }
}

impl ConsumedTrait for Consumed {
    fn or<U: FnOnce() -> Consumed>(self, other: U) -> Consumed {
        match self {
            Consumed::Consumed(_) => self,
            Consumed::Empty => other(),
        }
    }

    fn consume(amount: isize) -> Consumed {
        if amount == 0 {
            Consumed::Empty
        } else {
            Consumed::Consumed(amount)
        }
    }
}

impl ConsumeVar<isize> for Consumed {
    fn consume_var(&self, var: &mut isize) -> Consumed {
        match self {
            Consumed::Consumed(amount) => {
                *var += *amount;
                Consumed::Empty
            }
            Consumed::Empty => Consumed::Empty,
        }
    }
}

impl ConsumeVar<usize> for Consumed {
    fn consume_var(&self, var: &mut usize) -> Consumed {
        match self {
            Consumed::Consumed(amount) => {
                *var += *amount as usize;
                Consumed::Empty
            }
            Consumed::Empty => Consumed::Empty,
        }
    }
}