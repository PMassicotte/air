if (a) 1
if (a) 1 else 2
if (a) 1 else if (b) 2 else 3

# Spacing test
if(a)1 else if(b)2 else 3

# Line break test
if (a_really_really_long_condition_here_that_is_allowed_to_break_onto_the_next_line) 1 else 2

if (a) # becomes leading on `1 + 1`
{
  1 + 1
}

if (a) # becomes dangling on `{}`
{
}

if (a) # becomes dangling on `{}`
{
  # inner comment but empty `{}`
}

if (a) # becomes leading on `TRUE`
  TRUE

if (
  a
  # becomes trailing on `a`
) {
  TRUE
}

if (a # becomes trailing on `a`
) {
  TRUE
}

# Breaks, but the `condition` itself fits and is not expanded
if (this || this || this || this || this || this || this || this || this || this) {
  1
} else {
  2
}
# Breaks, but the `condition` itself also doesn't fit and is also expanded
if (this || this || this || this || this || this || this || this || this || this || this || this) {
  1
} else {
  2
}
