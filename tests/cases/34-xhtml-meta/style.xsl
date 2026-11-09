<?xml version="1.0"?>
<!-- An existing Content-Type meta is kept; an empty head gets one; the
     strict DOCTYPE by system ID only. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="xml" doctype-system="http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"/>
  <xsl:template match="/">
    <html xmlns="http://www.w3.org/1999/xhtml"><head><meta http-equiv="content-type" content="text/html"/></head><body><head/></body></html>
  </xsl:template>
</xsl:stylesheet>
