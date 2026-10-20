#!/bin/sh
cp tools/hooks/pre-commit .git/hooks/pre-commit && chmod +x .git/hooks/pre-commit
echo "pre-commit hook installed"
