const express = require('express');
const crypto = require('crypto');
const multer = require('multer');
const cors = require('cors');

const app = express();
const PORT = 5001;

// Middleware
app.use(cors());
app.use(express.json());

// In-memory storage for mock IPFS
const ipfsStorage = new Map();

// Configure multer for file uploads
const storage = multer.memoryStorage();
const upload = multer({ storage: storage });

// Generate a mock CID (Content Identifier)
function generateCID(content) {
  const hash = crypto.createHash('sha256').update(content).digest('hex');
  // Mock IPFS CID format (base58btc multihash)
  return `Qm${hash.substring(0, 44)}`;
}

// Mock IPFS API endpoint: /api/v0/add
app.post('/api/v0/add', upload.single('file'), (req, res) => {
  try {
    if (!req.file) {
      return res.status(400).json({ error: 'No file provided' });
    }

    const content = req.file.buffer;
    const cid = generateCID(content);

    // Store in memory
    ipfsStorage.set(cid, {
      content: content,
      contentType: req.file.mimetype || 'application/octet-stream',
      timestamp: new Date().toISOString(),
      size: content.length
    });

    console.log(`[IPFS] Stored ${content.length} bytes with CID: ${cid}`);

    // Return IPFS-like response
    res.json({
      Name: req.file.originalname || 'file',
      Hash: cid,
      Size: content.length.toString()
    });
  } catch (error) {
    console.error('[IPFS] Error:', error);
    res.status(500).json({ error: error.message });
  }
});

// Mock IPFS API endpoint: /api/v0/cat (retrieve content)
app.post('/api/v0/cat', express.text(), (req, res) => {
  try {
    const cid = req.query.arg || req.body;

    if (!cid) {
      return res.status(400).json({ error: 'CID not provided' });
    }

    const stored = ipfsStorage.get(cid);

    if (!stored) {
      return res.status(404).json({ error: 'CID not found' });
    }

    console.log(`[IPFS] Retrieved ${stored.size} bytes for CID: ${cid}`);

    res.setHeader('Content-Type', stored.contentType);
    res.send(stored.content);
  } catch (error) {
    console.error('[IPFS] Error:', error);
    res.status(500).json({ error: error.message });
  }
});

// Gateway endpoint: /ipfs/{cid}
app.get('/ipfs/:cid', (req, res) => {
  try {
    const cid = req.params.cid;
    const stored = ipfsStorage.get(cid);

    if (!stored) {
      return res.status(404).send('CID not found');
    }

    console.log(`[Gateway] Retrieved ${stored.size} bytes for CID: ${cid}`);

    res.setHeader('Content-Type', stored.contentType);
    res.send(stored.content);
  } catch (error) {
    console.error('[Gateway] Error:', error);
    res.status(500).json({ error: error.message });
  }
});

// Version endpoint
app.post('/api/v0/version', (req, res) => {
  res.json({
    Version: '0.13.0-mock',
    Commit: 'mock-ipfs-server',
    Repo: '11',
    System: 'amd64/linux',
    Golang: 'go1.19.1'
  });
});

app.get('/api/v0/version', (req, res) => {
  res.json({
    Version: '0.13.0-mock',
    Commit: 'mock-ipfs-server',
    Repo: '11',
    System: 'amd64/linux',
    Golang: 'go1.19.1'
  });
});

// Stats endpoint
app.get('/api/v0/stats', (req, res) => {
  res.json({
    NumObjects: ipfsStorage.size,
    RepoSize: Array.from(ipfsStorage.values()).reduce((sum, item) => sum + item.size, 0),
    RepoPath: '/mock/ipfs/repo',
    Version: '11',
    StorageMax: 10000000000
  });
});

// List all stored CIDs (debug endpoint)
app.get('/debug/list', (req, res) => {
  const list = Array.from(ipfsStorage.entries()).map(([cid, data]) => ({
    cid,
    size: data.size,
    timestamp: data.timestamp,
    contentType: data.contentType
  }));

  res.json({
    total: ipfsStorage.size,
    items: list
  });
});

// Health check
app.get('/health', (req, res) => {
  res.json({
    status: 'ok',
    stored_items: ipfsStorage.size,
    uptime: process.uptime()
  });
});

// Start server
app.listen(PORT, '0.0.0.0', () => {
  console.log('');
  console.log('========================================');
  console.log('   Mock IPFS Server Started');
  console.log('========================================');
  console.log(`Listening on: http://0.0.0.0:${PORT}`);
  console.log('');
  console.log('Available endpoints:');
  console.log(`  POST   /api/v0/add        - Upload content`);
  console.log(`  POST   /api/v0/cat        - Retrieve content by CID`);
  console.log(`  GET    /ipfs/:cid         - Gateway access`);
  console.log(`  POST   /api/v0/version    - Get version info`);
  console.log(`  GET    /debug/list        - List all stored CIDs`);
  console.log(`  GET    /health            - Health check`);
  console.log('');
  console.log('To use with offchain server, set:');
  console.log(`  IPFS_API_URL=http://127.0.0.1:${PORT}`);
  console.log('  IPFS_PROJECT_ID=');
  console.log('  IPFS_PROJECT_SECRET=');
  console.log('========================================');
  console.log('');
});

// Graceful shutdown
process.on('SIGINT', () => {
  console.log('\n[IPFS] Shutting down mock IPFS server...');
  process.exit(0);
});

process.on('SIGTERM', () => {
  console.log('\n[IPFS] Shutting down mock IPFS server...');
  process.exit(0);
});
