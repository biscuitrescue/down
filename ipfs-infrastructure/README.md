# IPFS Infrastructure for Oilseed Value Chain

This directory contains the complete IPFS infrastructure setup for the Oilseed Value Chain supply chain management system. It provides a production-ready, scalable, and highly available IPFS deployment with monitoring, backup, and disaster recovery capabilities.

## 🏗️ Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    IPFS DEPLOYMENT STACK                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   Grafana   │  │ Prometheus  │  │    Health Checker       │  │
│  │   :3001     │  │    :9090    │  │   (Node.js Service)     │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │               IPFS Core Services                            │ │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │ │
│  │  │IPFS Gateway │  │ IPFS Node   │  │   IPFS Cluster      │ │ │
│  │  │(Nginx:8081)│  │   :5001     │  │      :9094          │ │ │
│  │  └─────────────┘  └─────────────┘  └─────────────────────┘ │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │              Supporting Services                            │ │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │ │
│  │  │PostgreSQL   │  │   Redis     │  │   Pin Manager       │ │ │
│  │  │   :5432     │  │   :6379     │  │  (Node.js Service)  │ │ │
│  │  └─────────────┘  └─────────────┘  └─────────────────────┘ │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## 🚀 Quick Start

### Prerequisites

- **Docker** (version 20.0+ recommended)
- **Docker Compose** (version 2.0+ recommended)
- **Minimum 4GB RAM** and **20GB disk space**
- **Network connectivity** for pulling container images

### Development Deployment

```bash
# Clone the repository
git clone <repository-url>
cd sih-25/ipfs-infrastructure

# Deploy development environment
./deploy.sh development
```

### Production Deployment

```bash
# Set up environment variables (see Configuration section)
cp ../offchain/.env.production .env

# Deploy production environment
./deploy.sh production
```

## 📋 Service Endpoints

After deployment, the following services will be available:

| Service | URL | Description |
|---------|-----|-------------|
| IPFS API | http://localhost:5001 | Core IPFS API for uploads/downloads |
| IPFS Gateway | http://localhost:8081 | HTTP gateway with caching |
| IPFS Cluster | http://localhost:9094 | Cluster management API |
| PostgreSQL | localhost:5432 | Metadata database |
| Redis | localhost:6379 | Caching and session storage |
| Prometheus | http://localhost:9090 | Metrics collection |
| Grafana | http://localhost:3001 | Monitoring dashboards |

## ⚙️ Configuration

### Environment Variables

Create a `.env` file with the following variables:

```bash
# IPFS Configuration
IPFS_API_URL=http://127.0.0.1:5001
IPFS_GATEWAY_URL=http://127.0.0.1:8081/ipfs/
IPFS_TIMEOUT_SECONDS=30
IPFS_MAX_FILE_SIZE_MB=100
IPFS_RETRY_ATTEMPTS=3

# External IPFS Services (for redundancy)
INFURA_PROJECT_ID=your_infura_project_id
INFURA_PROJECT_SECRET=your_infura_secret
PINATA_API_KEY=your_pinata_api_key
PINATA_SECRET_KEY=your_pinata_secret

# Database
DATABASE_URL=postgresql://ipfs_user:password@localhost:5432/ipfs_metadata
REDIS_URL=redis://localhost:6379

# Security
CLUSTER_SECRET=your_32_byte_hex_secret

# Monitoring
SLACK_WEBHOOK_URL=your_slack_webhook_for_alerts
```

### External Service Setup

#### Infura IPFS (Primary Production Service)

1. Sign up at [infura.io](https://infura.io)
2. Create a new IPFS project
3. Copy Project ID and Secret to your `.env` file

#### Pinata (Backup Service)

1. Sign up at [pinata.cloud](https://pinata.cloud)
2. Generate API keys
3. Copy keys to your `.env` file

## 🏗️ Services Overview

### Core IPFS Services

#### IPFS Node (`ipfs-node`)
- **Purpose**: Primary IPFS daemon for content storage and retrieval
- **Image**: `ipfs/kubo:latest`
- **Ports**: 4001 (P2P), 5001 (API), 8080 (Gateway)
- **Storage**: Persistent volume for IPFS data

#### IPFS Cluster (`ipfs-cluster`)
- **Purpose**: Distributed pinning and replication
- **Image**: `ipfs/ipfs-cluster:latest`
- **Ports**: 9094 (API), 9095 (Proxy), 9096 (Swarm)
- **Features**: Automatic failover, load balancing

#### IPFS Gateway (`ipfs-gateway`)
- **Purpose**: HTTP gateway with caching and load balancing
- **Image**: `nginx:alpine`
- **Port**: 8081
- **Features**: Content caching, gzip compression, rate limiting

### Supporting Services

#### PostgreSQL (`postgres`)
- **Purpose**: Metadata storage and indexing
- **Schema**: Supply chain entities, IPFS content registry, audit logs
- **Features**: Full-text search, JSON queries, spatial data support

#### Redis (`redis`)
- **Purpose**: Caching and session management
- **Configuration**: Optimized for IPFS metadata caching
- **Features**: Persistence, memory optimization

#### Pin Manager (`pin-manager`)
- **Purpose**: Automated pinning to external services
- **Language**: Node.js
- **Features**: 
  - Automatic backup to Pinata/Infura
  - Pin status monitoring
  - Failed pin retry logic

#### Health Checker (`health-checker`)
- **Purpose**: Service monitoring and alerting
- **Language**: Node.js
- **Features**: 
  - Continuous health monitoring
  - Slack alerts for failures
  - Performance metrics collection

### Monitoring Stack

#### Prometheus (`prometheus`)
- **Purpose**: Metrics collection and alerting
- **Targets**: All IPFS and supporting services
- **Retention**: 30 days of metrics data

#### Grafana (`grafana`)
- **Purpose**: Monitoring dashboards and visualization
- **Default Login**: admin / admin_change_in_production
- **Features**: Pre-configured dashboards for IPFS metrics

## 📊 Data Structure

### IPFS Content Organization

Each piece of content stored in IPFS follows a standardized structure:

```json
{
  "metadata": {
    "version": "1.0",
    "timestamp": "2024-01-15T10:30:00Z",
    "stage": "farmer_registration",
    "content_hash": "abc123...",
    "size_bytes": 1024,
    "upload_source": "rust-backend"
  },
  "content": {
    // Stage-specific content
  }
}
```

### Supply Chain Stages

1. **Farmer Registration** (`farmer_registration`)
   - Farmer profile and verification documents
   - Land certificates and satellite imagery
   - Crop information and certifications

2. **FPO Purchase** (`fpo_purchase`)
   - Purchase receipts and quality reports
   - Batch information and weighment slips
   - Photo documentation

3. **Warehouse Storage** (`warehouse_storage`)
   - IoT sensor data and storage conditions
   - Fumigation certificates and inspections
   - Inventory management records

4. **Logistics** (`logistics`)
   - GPS tracking data and route information
   - Temperature logs and delivery confirmations
   - Transportation documents

5. **Processing** (`processing`)
   - Lab reports and yield calculations
   - Processing parameters and quality checks
   - Transformation documentation

6. **Packaging** (`packaging`)
   - SKU information and unit details
   - Label designs and barcode data
   - Merkle tree proofs for traceability

7. **AI Scoring** (`ai_scoring`)
   - Model inputs and scoring results
   - Explainability reports and recommendations
   - Quality and trust score calculations

## 🔧 Management Commands

### Service Management

```bash
# View all service logs
docker-compose -p oilseed-ipfs logs -f

# View specific service logs
docker-compose -p oilseed-ipfs logs -f ipfs-node

# Restart a service
docker-compose -p oilseed-ipfs restart ipfs-node

# Scale cluster nodes
docker-compose -p oilseed-ipfs up -d --scale ipfs-cluster=3

# Stop all services
docker-compose -p oilseed-ipfs down

# Stop and remove all data (DESTRUCTIVE)
docker-compose -p oilseed-ipfs down -v
```

### IPFS Operations

```bash
# Check IPFS node status
curl http://localhost:5001/api/v0/version

# List pinned content
curl -X POST http://localhost:5001/api/v0/pin/ls

# Check cluster status
curl http://localhost:9094/version

# View cluster peers
curl http://localhost:9094/peers
```

### Database Operations

```bash
# Connect to PostgreSQL
docker exec -it ipfs-postgres psql -U ipfs_user -d ipfs_metadata

# View content summary
docker exec -it ipfs-postgres psql -U ipfs_user -d ipfs_metadata -c "SELECT * FROM ipfs_content_summary;"

# Backup database
docker exec ipfs-postgres pg_dump -U ipfs_user ipfs_metadata > backup.sql
```

## 📈 Monitoring and Alerting

### Prometheus Metrics

Available at `http://localhost:9090/metrics`:

- IPFS node metrics (peers, storage, pins)
- Cluster health and replication status
- Database connection pools and query performance
- Redis cache hit/miss ratios
- HTTP gateway response times

### Grafana Dashboards

Access Grafana at `http://localhost:3001` with admin/admin:

1. **IPFS Overview Dashboard**
   - Node status and peer connections
   - Storage usage and pin counts
   - Gateway performance metrics

2. **Supply Chain Dashboard**
   - Content uploads by stage
   - Processing times and success rates
   - Geographic distribution of data

3. **Infrastructure Dashboard**
   - System resource usage
   - Database performance
   - Alert status and history

### Alerts Configuration

Configure alerts for:
- IPFS node disconnection
- High storage usage (>80%)
- Failed pin operations
- Database connection issues
- High response times (>5s)

## 🔒 Security Considerations

### Network Security

1. **Firewall Rules**: Only expose necessary ports
2. **TLS Termination**: Use reverse proxy with SSL certificates
3. **API Authentication**: Implement API keys for administrative endpoints
4. **Rate Limiting**: Configure rate limits on public endpoints

### Data Security

1. **Encryption**: Encrypt sensitive data before IPFS upload
2. **Access Control**: Role-based access to administrative functions
3. **Audit Logging**: Comprehensive audit trails for all operations
4. **Backup Encryption**: Encrypt all backups

### Operational Security

1. **Secret Management**: Use external secret management systems
2. **Regular Updates**: Keep all container images updated
3. **Vulnerability Scanning**: Regular security scans
4. **Incident Response**: Documented incident response procedures

## 🔄 Backup and Recovery

### Automated Backups

1. **IPFS Data**: Automatic replication to external services
2. **PostgreSQL**: Daily database dumps to S3-compatible storage
3. **Configuration**: Version-controlled configuration files
4. **Cluster State**: Regular cluster state exports

### Recovery Procedures

1. **Service Recovery**: 
   ```bash
   # Restore from backup
   ./deploy.sh production --restore-from-backup backup_20240115
   ```

2. **Data Recovery**:
   ```bash
   # Restore database from backup
   docker exec -i ipfs-postgres psql -U ipfs_user -d ipfs_metadata < backup.sql
   ```

3. **Disaster Recovery**: Full documentation in `disaster-recovery.md`

## 🧪 Testing

### Health Checks

```bash
# Run comprehensive health check
./scripts/health-check.sh

# Test IPFS upload/download
./scripts/test-ipfs.sh

# Test database connectivity
./scripts/test-database.sh
```

### Load Testing

```bash
# Install k6 load testing tool
# Run IPFS load tests
k6 run tests/load-test.js
```

### Integration Tests

```bash
# Run integration test suite
npm test
```

## 🚀 Production Deployment Checklist

Before deploying to production:

- [ ] Environment variables configured
- [ ] SSL certificates installed
- [ ] Firewall rules configured
- [ ] External services (Infura/Pinata) configured
- [ ] Backup procedures tested
- [ ] Monitoring alerts configured
- [ ] Load testing completed
- [ ] Security audit performed
- [ ] Documentation updated
- [ ] Incident response plan ready

## 🛠️ Troubleshooting

### Common Issues

#### IPFS Node Won't Start
```bash
# Check logs
docker-compose -p oilseed-ipfs logs ipfs-node

# Common solutions:
# 1. Insufficient disk space
# 2. Port conflicts
# 3. Corrupted data directory
```

#### High Memory Usage
```bash
# Check resource usage
docker stats

# Solutions:
# 1. Adjust garbage collection settings
# 2. Increase available memory
# 3. Configure connection limits
```

#### Slow Performance
```bash
# Check gateway cache hit ratio
curl http://localhost:8081/nginx_status

# Solutions:
# 1. Increase cache size
# 2. Add more cluster nodes
# 3. Optimize database queries
```

### Log Locations

- **Service Logs**: `docker-compose logs [service]`
- **IPFS Logs**: `./data/ipfs/logs/`
- **Application Logs**: `./logs/`
- **Nginx Logs**: `/var/log/nginx/` (inside container)

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make changes and test thoroughly
4. Update documentation
5. Submit a pull request

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 📞 Support

For issues and questions:

1. Check the [Troubleshooting](#troubleshooting) section
2. Review existing GitHub issues
3. Create a new issue with detailed information
4. Contact the development team

## 🗺️ Roadmap

### Short Term (Q1 2024)
- [ ] Multi-region deployment support
- [ ] Advanced caching strategies
- [ ] Enhanced monitoring dashboards
- [ ] Automated scaling

### Long Term (Q2-Q3 2024)
- [ ] Kubernetes deployment
- [ ] Cross-cloud replication
- [ ] Advanced analytics
- [ ] Machine learning integration

---

**Last Updated**: January 2024
**Version**: 1.0.0
**Maintainers**: Oilseed Value Chain Development Team