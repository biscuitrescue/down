#!/bin/bash

# IPFS Infrastructure Deployment Script
# Production-ready deployment for Oilseed Value Chain IPFS Architecture

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
DEPLOYMENT_ENV="${1:-development}"
DOCKER_COMPOSE_FILE="docker-compose.yml"
PROJECT_NAME="oilseed-ipfs"
BACKUP_DIR="./backups"
LOG_DIR="./logs"

# Functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

check_dependencies() {
    log_step "Checking dependencies..."

    # Check Docker
    if ! command -v docker &> /dev/null; then
        log_error "Docker is not installed. Please install Docker first."
        exit 1
    fi

    # Check Docker Compose
    if ! command -v docker-compose &> /dev/null && ! docker compose version &> /dev/null; then
        log_error "Docker Compose is not installed. Please install Docker Compose first."
        exit 1
    fi

    # Check if running as root (not recommended)
    if [[ $EUID -eq 0 ]]; then
        log_warn "Running as root is not recommended for production deployments."
    fi

    log_info "Dependencies check passed"
}

create_directories() {
    log_step "Creating necessary directories..."

    mkdir -p "$BACKUP_DIR"
    mkdir -p "$LOG_DIR"
    mkdir -p "./data/ipfs"
    mkdir -p "./data/cluster"
    mkdir -p "./data/postgres"
    mkdir -p "./data/redis"
    mkdir -p "./data/prometheus"
    mkdir -p "./data/grafana"
    mkdir -p "./config"
    mkdir -p "./certs"

    log_info "Directories created successfully"
}

generate_configs() {
    log_step "Generating configuration files..."

    # Generate Nginx config
    cat > nginx.conf << 'EOF'
events {
    worker_connections 1024;
}

http {
    upstream ipfs_backend {
        server ipfs-node:8080;
    }

    proxy_cache_path /var/cache/nginx levels=1:2 keys_zone=ipfs_cache:10m max_size=1g inactive=60m use_temp_path=off;

    server {
        listen 80;
        server_name _;

        # Health check endpoint
        location /health {
            access_log off;
            return 200 "OK\n";
            add_header Content-Type text/plain;
        }

        # IPFS Gateway with caching
        location /ipfs/ {
            proxy_pass http://ipfs_backend;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;

            # Caching settings
            proxy_cache ipfs_cache;
            proxy_cache_valid 200 1h;
            proxy_cache_valid 404 1m;
            proxy_cache_use_stale error timeout updating http_500 http_502 http_503 http_504;

            # Add cache headers
            add_header X-Cache-Status $upstream_cache_status;

            # Timeout settings
            proxy_connect_timeout 30s;
            proxy_send_timeout 30s;
            proxy_read_timeout 30s;
        }

        # API endpoints (no caching)
        location /api/ {
            proxy_pass http://ipfs_backend;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;

            # No caching for API calls
            proxy_no_cache 1;
            proxy_cache_bypass 1;
        }
    }
}
EOF

    # Generate Prometheus config
    cat > prometheus.yml << 'EOF'
global:
  scrape_interval: 15s
  evaluation_interval: 15s

rule_files:
  # - "first_rules.yml"
  # - "second_rules.yml"

scrape_configs:
  - job_name: 'prometheus'
    static_configs:
      - targets: ['localhost:9090']

  - job_name: 'ipfs-node'
    static_configs:
      - targets: ['ipfs-node:5001']
    metrics_path: '/debug/metrics/prometheus'

  - job_name: 'ipfs-cluster'
    static_configs:
      - targets: ['ipfs-cluster:9094']
    metrics_path: '/metrics'

  - job_name: 'postgres'
    static_configs:
      - targets: ['postgres:5432']

  - job_name: 'redis'
    static_configs:
      - targets: ['redis:6379']

  - job_name: 'nginx'
    static_configs:
      - targets: ['ipfs-gateway:80']

alerting:
  alertmanagers:
    - static_configs:
        - targets:
          # - alertmanager:9093
EOF

    # Generate cluster secret
    if [ ! -f "./config/cluster-secret" ]; then
        openssl rand -hex 32 > ./config/cluster-secret
        log_info "Generated IPFS cluster secret"
    fi

    log_info "Configuration files generated successfully"
}

setup_monitoring() {
    log_step "Setting up monitoring configurations..."

    # Create Grafana directories
    mkdir -p ./grafana/dashboards
    mkdir -p ./grafana/datasources

    # Generate Grafana datasource config
    cat > ./grafana/datasources/prometheus.yml << 'EOF'
apiVersion: 1

datasources:
  - name: Prometheus
    type: prometheus
    access: proxy
    url: http://prometheus:9090
    isDefault: true
    editable: true
EOF

    # Generate Grafana dashboard config
    cat > ./grafana/dashboards/dashboard.yml << 'EOF'
apiVersion: 1

providers:
  - name: 'default'
    orgId: 1
    folder: ''
    type: file
    disableDeletion: false
    updateIntervalSeconds: 10
    allowUiUpdates: true
    options:
      path: /etc/grafana/provisioning/dashboards
EOF

    log_info "Monitoring configuration setup completed"
}

create_pin_manager() {
    log_step "Creating IPFS pin manager service..."

    mkdir -p ./pin-manager

    cat > ./pin-manager/package.json << 'EOF'
{
  "name": "ipfs-pin-manager",
  "version": "1.0.0",
  "description": "IPFS Pin Management Service for Oilseed Value Chain",
  "main": "index.js",
  "scripts": {
    "start": "node index.js",
    "dev": "nodemon index.js"
  },
  "dependencies": {
    "axios": "^1.6.0",
    "pg": "^8.11.0",
    "redis": "^4.6.0",
    "cron": "^3.1.0",
    "winston": "^3.11.0"
  },
  "engines": {
    "node": ">=18.0.0"
  }
}
EOF

    cat > ./pin-manager/index.js << 'EOF'
const axios = require('axios');
const { Client } = require('pg');
const redis = require('redis');
const cron = require('cron');
const winston = require('winston');

// Configure logger
const logger = winston.createLogger({
  level: 'info',
  format: winston.format.json(),
  defaultMeta: { service: 'pin-manager' },
  transports: [
    new winston.transports.Console({
      format: winston.format.simple()
    })
  ]
});

class PinManager {
  constructor() {
    this.ipfsUrl = process.env.IPFS_API_URL || 'http://ipfs-node:5001';
    this.postgresUrl = process.env.POSTGRES_URL;
    this.redisUrl = process.env.REDIS_URL;
    this.infuraProjectId = process.env.INFURA_PROJECT_ID;
    this.infuraProjectSecret = process.env.INFURA_PROJECT_SECRET;
    this.pinataApiKey = process.env.PINATA_API_KEY;
    this.pinataSecretKey = process.env.PINATA_SECRET_KEY;

    this.setupDatabases();
    this.startCronJobs();
  }

  async setupDatabases() {
    // Setup PostgreSQL
    if (this.postgresUrl) {
      this.postgres = new Client({ connectionString: this.postgresUrl });
      await this.postgres.connect();
      logger.info('Connected to PostgreSQL');
    }

    // Setup Redis
    if (this.redisUrl) {
      this.redis = redis.createClient({ url: this.redisUrl });
      await this.redis.connect();
      logger.info('Connected to Redis');
    }
  }

  startCronJobs() {
    // Check pin status every 5 minutes
    const pinCheckJob = new cron.CronJob('*/5 * * * *', async () => {
      await this.checkPinStatus();
    });

    // Backup pins to external services every hour
    const backupJob = new cron.CronJob('0 * * * *', async () => {
      await this.backupToExternalServices();
    });

    pinCheckJob.start();
    backupJob.start();

    logger.info('Cron jobs started');
  }

  async checkPinStatus() {
    try {
      const response = await axios.post(`${this.ipfsUrl}/api/v0/pin/ls`, {}, {
        params: { type: 'recursive' }
      });

      logger.info(`Pin status check completed. Active pins: ${Object.keys(response.data.Keys || {}).length}`);
    } catch (error) {
      logger.error('Pin status check failed:', error.message);
    }
  }

  async backupToExternalServices() {
    logger.info('Starting backup to external services...');

    try {
      // Get list of pins from local IPFS
      const pinsResponse = await axios.post(`${this.ipfsUrl}/api/v0/pin/ls`, {}, {
        params: { type: 'recursive' }
      });

      const pins = Object.keys(pinsResponse.data.Keys || {});

      for (const cid of pins.slice(0, 10)) { // Limit to 10 per batch
        await this.pinToExternal(cid);
        await new Promise(resolve => setTimeout(resolve, 1000)); // Rate limiting
      }

      logger.info(`Backup completed for ${pins.slice(0, 10).length} pins`);
    } catch (error) {
      logger.error('Backup to external services failed:', error.message);
    }
  }

  async pinToExternal(cid) {
    // Pin to Pinata if configured
    if (this.pinataApiKey && this.pinataSecretKey) {
      try {
        await axios.post('https://api.pinata.cloud/pinning/pinByHash', {
          hashToPin: cid,
          pinataMetadata: {
            name: `oilseed-backup-${cid}`,
            keyvalues: {
              service: 'oilseed-value-chain',
              timestamp: new Date().toISOString()
            }
          }
        }, {
          headers: {
            'pinata_api_key': this.pinataApiKey,
            'pinata_secret_api_key': this.pinataSecretKey
          }
        });

        logger.info(`Successfully pinned ${cid} to Pinata`);
      } catch (error) {
        logger.warn(`Failed to pin ${cid} to Pinata:`, error.message);
      }
    }
  }
}

// Start the service
const pinManager = new PinManager();

// Graceful shutdown
process.on('SIGINT', async () => {
  logger.info('Shutting down pin manager...');
  process.exit(0);
});

logger.info('IPFS Pin Manager started');
EOF

    log_info "Pin manager service created"
}

create_health_checker() {
    log_step "Creating health check service..."

    mkdir -p ./health-checker

    cat > ./health-checker/package.json << 'EOF'
{
  "name": "ipfs-health-checker",
  "version": "1.0.0",
  "description": "Health monitoring service for IPFS infrastructure",
  "main": "health-check.js",
  "scripts": {
    "start": "node health-check.js"
  },
  "dependencies": {
    "axios": "^1.6.0",
    "winston": "^3.11.0"
  }
}
EOF

    cat > ./health-checker/health-check.js << 'EOF'
const axios = require('axios');
const winston = require('winston');

const logger = winston.createLogger({
  level: 'info',
  format: winston.format.json(),
  defaultMeta: { service: 'health-checker' },
  transports: [
    new winston.transports.Console({
      format: winston.format.simple()
    })
  ]
});

class HealthChecker {
  constructor() {
    this.ipfsUrl = process.env.IPFS_API_URL || 'http://ipfs-node:5001';
    this.clusterUrl = process.env.CLUSTER_API_URL || 'http://ipfs-cluster:9094';
    this.checkInterval = parseInt(process.env.CHECK_INTERVAL) || 60000;
    this.slackWebhook = process.env.SLACK_WEBHOOK_URL;

    this.startHealthChecks();
  }

  startHealthChecks() {
    setInterval(async () => {
      await this.performHealthChecks();
    }, this.checkInterval);

    logger.info(`Health checker started with ${this.checkInterval}ms interval`);
  }

  async performHealthChecks() {
    const results = {
      ipfs: await this.checkIPFS(),
      cluster: await this.checkCluster(),
      timestamp: new Date().toISOString()
    };

    logger.info('Health check results:', results);

    // Send alerts if any service is down
    const failedServices = Object.entries(results)
      .filter(([key, value]) => key !== 'timestamp' && !value)
      .map(([key]) => key);

    if (failedServices.length > 0) {
      await this.sendAlert(`Services down: ${failedServices.join(', ')}`);
    }
  }

  async checkIPFS() {
    try {
      const response = await axios.post(`${this.ipfsUrl}/api/v0/version`, {}, {
        timeout: 5000
      });
      return response.status === 200;
    } catch (error) {
      logger.error('IPFS health check failed:', error.message);
      return false;
    }
  }

  async checkCluster() {
    try {
      const response = await axios.get(`${this.clusterUrl}/version`, {
        timeout: 5000
      });
      return response.status === 200;
    } catch (error) {
      logger.error('Cluster health check failed:', error.message);
      return false;
    }
  }

  async sendAlert(message) {
    if (this.slackWebhook) {
      try {
        await axios.post(this.slackWebhook, {
          text: `🚨 IPFS Infrastructure Alert: ${message}`,
          username: 'IPFS Health Checker'
        });
        logger.info('Alert sent to Slack');
      } catch (error) {
        logger.error('Failed to send Slack alert:', error.message);
      }
    }
  }
}

new HealthChecker();
EOF

    log_info "Health checker service created"
}

setup_ssl() {
    log_step "Setting up SSL certificates..."

    if [ "$DEPLOYMENT_ENV" = "production" ]; then
        log_warn "Production SSL setup requires manual configuration with your domain and certificate authority"
        log_warn "Please configure SSL certificates in ./certs/ directory before production deployment"
    else
        log_info "Development environment - SSL setup skipped"
    fi
}

validate_environment() {
    log_step "Validating environment configuration..."

    if [ "$DEPLOYMENT_ENV" = "production" ]; then
        # Check for production environment variables
        if [ -z "${INFURA_PROJECT_ID:-}" ] && [ -z "${PINATA_API_KEY:-}" ]; then
            log_error "Production deployment requires IPFS service credentials (Infura or Pinata)"
            exit 1
        fi

        log_warn "Production deployment detected. Please ensure:"
        log_warn "1. Environment variables are properly set"
        log_warn "2. SSL certificates are configured"
        log_warn "3. Firewall rules are in place"
        log_warn "4. Backup procedures are established"
    fi

    log_info "Environment validation completed"
}

deploy_services() {
    log_step "Deploying IPFS infrastructure services..."

    # Export cluster secret
    export CLUSTER_SECRET=$(cat ./config/cluster-secret)

    # Start services
    if command -v docker-compose &> /dev/null; then
        docker-compose -f "$DOCKER_COMPOSE_FILE" -p "$PROJECT_NAME" up -d
    else
        docker compose -f "$DOCKER_COMPOSE_FILE" -p "$PROJECT_NAME" up -d
    fi

    log_info "Services deployment initiated"
}

wait_for_services() {
    log_step "Waiting for services to become healthy..."

    local max_attempts=30
    local attempt=1

    while [ $attempt -le $max_attempts ]; do
        if docker ps --format "table {{.Names}}\t{{.Status}}" | grep -q "healthy\|Up"; then
            log_info "Services are starting up... (attempt $attempt/$max_attempts)"
        fi

        # Check specific services
        local ipfs_healthy=false
        local postgres_healthy=false

        if docker exec ipfs-node ipfs id &>/dev/null; then
            ipfs_healthy=true
        fi

        if docker exec ipfs-postgres pg_isready -U ipfs_user -d ipfs_metadata &>/dev/null; then
            postgres_healthy=true
        fi

        if [ "$ipfs_healthy" = true ] && [ "$postgres_healthy" = true ]; then
            log_info "Core services are healthy"
            break
        fi

        if [ $attempt -eq $max_attempts ]; then
            log_error "Services failed to become healthy within timeout"
            exit 1
        fi

        sleep 10
        ((attempt++))
    done
}

run_post_deployment_tests() {
    log_step "Running post-deployment tests..."

    # Test IPFS API
    if curl -s http://localhost:5001/api/v0/version > /dev/null; then
        log_info "✓ IPFS API is accessible"
    else
        log_error "✗ IPFS API test failed"
    fi

    # Test IPFS Gateway
    if curl -s http://localhost:8081/health > /dev/null; then
        log_info "✓ IPFS Gateway is accessible"
    else
        log_error "✗ IPFS Gateway test failed"
    fi

    # Test Cluster API
    if curl -s http://localhost:9094/version > /dev/null; then
        log_info "✓ IPFS Cluster API is accessible"
    else
        log_warn "⚠ IPFS Cluster API test failed (may still be starting)"
    fi

    # Test monitoring
    if curl -s http://localhost:9090/api/v1/status/config > /dev/null; then
        log_info "✓ Prometheus is accessible"
    else
        log_warn "⚠ Prometheus test failed"
    fi

    if curl -s http://localhost:3001/api/health > /dev/null; then
        log_info "✓ Grafana is accessible"
    else
        log_warn "⚠ Grafana test failed"
    fi

    log_info "Post-deployment tests completed"
}

print_deployment_info() {
    log_step "Deployment Summary"

    echo ""
    echo "================== IPFS INFRASTRUCTURE DEPLOYED =================="
    echo ""
    echo "Environment: $DEPLOYMENT_ENV"
    echo "Project Name: $PROJECT_NAME"
    echo ""
    echo "Service Endpoints:"
    echo "  IPFS API:           http://localhost:5001"
    echo "  IPFS Gateway:       http://localhost:8081"
    echo "  IPFS Cluster API:   http://localhost:9094"
    echo "  PostgreSQL:         localhost:5432"
    echo "  Redis:              localhost:6379"
    echo "  Prometheus:         http://localhost:9090"
    echo "  Grafana:            http://localhost:3001"
    echo ""
    echo "Default Credentials:"
    echo "  Grafana Admin:      admin / admin_change_in_production"
    echo "  PostgreSQL:         ipfs_user / secure_password_change_in_production"
    echo ""
    echo "Management Commands:"
    echo "  View logs:          docker-compose -p $PROJECT_NAME logs -f [service]"
    echo "  Stop services:      docker-compose -p $PROJECT_NAME down"
    echo "  Restart service:    docker-compose -p $PROJECT_NAME restart [service]"
    echo "  Scale cluster:      docker-compose -p $PROJECT_NAME up -d --scale ipfs-cluster=3"
    echo ""
    echo "Important Files:"
    echo "  Cluster Secret:     ./config/cluster-secret"
    echo "  Nginx Config:       ./nginx.conf"
    echo "  Prometheus Config:  ./prometheus.yml"
    echo "  Data Directory:     ./data/"
    echo "  Logs Directory:     ./logs/"
    echo ""

    if [ "$DEPLOYMENT_ENV" = "production" ]; then
        echo "PRODUCTION DEPLOYMENT NOTES:"
        echo "  ⚠ Change default passwords before going live"
        echo "  ⚠ Configure SSL certificates"
        echo "  ⚠ Set up firewall rules"
        echo "  ⚠ Configure backup procedures"
        echo "  ⚠ Set up monitoring alerts"
    fi

    echo ""
    echo "================================================================="
}

backup_existing_deployment() {
    if docker ps -a | grep -q "$PROJECT_NAME"; then
        log_step "Backing up existing deployment..."

        local backup_timestamp=$(date +%Y%m%d_%H%M%S)
        local backup_path="$BACKUP_DIR/deployment_backup_$backup_timestamp"

        mkdir -p "$backup_path"

        # Backup data
        if [ -d "./data" ]; then
            cp -r ./data "$backup_path/"
            log_info "Data backed up to $backup_path/data"
        fi

        # Backup configurations
        if [ -f "./config/cluster-secret" ]; then
            cp -r ./config "$backup_path/"
            log_info "Config backed up to $backup_path/config"
        fi

        log_info "Existing deployment backed up to $backup_path"
    fi
}

cleanup_old_resources() {
    log_step "Cleaning up old resources..."

    # Stop existing containers
    if command -v docker-compose &> /dev/null; then
        docker-compose -p "$PROJECT_NAME" down --remove-orphans 2>/dev/null || true
    else
        docker compose -p "$PROJECT_NAME" down --remove-orphans 2>/dev/null || true
    fi

    # Clean up unused Docker resources
    docker system prune -f --volumes 2>/dev/null || true

    log_info "Cleanup completed"
}

# Main deployment flow
main() {
    echo ""
    log_info "Starting IPFS Infrastructure Deployment"
    log_info "Environment: $DEPLOYMENT_ENV"
    echo ""

    check_dependencies
    backup_existing_deployment
    cleanup_old_resources
    create_directories
    generate_configs
    setup_monitoring
    create_pin_manager
    create_health_checker
    setup_ssl
    validate_environment
    deploy_services
    wait_for_services
    run_post_deployment_tests
    print_deployment_info

    log_info "IPFS Infrastructure deployment completed successfully!"
}

# Help function
show_help() {
    echo "IPFS Infrastructure Deployment Script"
    echo ""
    echo "Usage: $0 [ENVIRONMENT]"
    echo ""
    echo "Environments:"
    echo "  development  - Local development setup (default)"
    echo "  staging      - Staging environment setup"
    echo "  production   - Production environment setup"
    echo ""
    echo "Examples:"
    echo "  $0                    # Deploy development environment"
    echo "  $0 development        # Deploy development environment"
    echo "  $0 production         # Deploy production environment"
    echo ""
    echo "Requirements:"
    echo "  - Docker and Docker Compose installed"
    echo "  - Sufficient disk space (5GB+ recommended)"
    echo "  - Network connectivity for pulling images"
    echo ""
}

# Handle command line arguments
case "${1:-}" in
    -h|--help)
        show_help
        exit 0
        ;;
    development|staging|production)
        main
        ;;
    "")
        main
        ;;
    *)
        log_error "Unknown environment: $1"
        show_help
        exit 1
        ;;
esac
