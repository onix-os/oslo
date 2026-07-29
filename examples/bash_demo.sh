#!/usr/bin/env sh

echo "=== Running Complex .sh Script with oslo ==="

# Variable assignments & arithmetic
A=15
B=27
SUM=$((A + B))
echo "SUM of $A and $B is $SUM"

# Shell function
say_hello() {
    echo "Hello from shell function: $1"
}

say_hello "OsloShell"

# If-Else Statement
if [ $SUM -gt 30 ]; then
    echo "Sum is greater than 30!"
else
    echo "Sum is smaller!"
fi

# Pipeline
echo "apple banana cherry" | grep banana

# For loop
for fruit in apple banana cherry; do
    echo "Fruit: $fruit"
done
