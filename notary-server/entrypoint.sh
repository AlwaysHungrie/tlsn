#!/bin/sh

echo "Starting entrypoint script"
echo "ENV is set to: $ENV"

# Check the value of the ENV variable
if [ "$ENV" = "dev" ]; then
    echo "🟡 Running in development mode ahi"
    echo $AIRDROP_SERVER_AUTH
    exec notary-server --config-file ~/.notary-server/config/config_dev.yml
else
    echo "🟢 Running in production mode"
    exec notary-server --config-file ~/.notary-server/config/config_prod.yml
fi